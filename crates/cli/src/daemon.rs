//! Finding, starting and stopping the daemon from the command line.
//!
//! The daemon is this same executable running `equipe daemon run`, detached from
//! the terminal that started it: it gets a session of its own (`setsid`), so
//! closing that terminal never reaches it, and its output goes to
//! `~/.equipe/daemon.log`.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use equipe_core::gateway::{self, Client};
use equipe_core::protocol::rpc::Hello;
use equipe_core::runtime;
use equipe_providers::credentials::equipe_home;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Rotate the log past this size so a long-lived daemon never fills the disk.
const LOG_LIMIT: u64 = 5 * 1024 * 1024;

pub fn socket() -> PathBuf {
    gateway::socket_path(&equipe_home())
}

/// Which build this executable is: the version plus when it was built, so a
/// daemon left running from an older build of the same version is noticed.
pub fn build_id() -> String {
    let built = std::env::current_exe()
        .and_then(std::fs::metadata)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    format!("{VERSION}+{built}")
}

/// `equipe daemon run`: serve in the foreground until asked to stop.
pub async fn run_foreground() -> io::Result<()> {
    let config = runtime::Config::for_user(VERSION).persistent();
    gateway::serve(config, &socket(), &build_id()).await
}

/// Start a daemon in the background unless one is answering. `Ok(true)` when
/// this call started it.
pub async fn ensure_running() -> io::Result<bool> {
    let socket = socket();
    if gateway::is_running(&socket).await {
        return Ok(false);
    }
    spawn_detached()?;
    for _ in 0..100 {
        if gateway::is_running(&socket).await {
            return Ok(true);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!("the daemon didn't start — see {}", log_path().display()),
    ))
}

fn log_path() -> PathBuf {
    equipe_home().join("daemon.log")
}

fn spawn_detached() -> io::Result<()> {
    use std::os::unix::process::CommandExt as _;
    use std::process::{Command, Stdio};

    let home = equipe_home();
    std::fs::create_dir_all(&home)?;
    let log = log_path();
    if std::fs::metadata(&log).is_ok_and(|m| m.len() > LOG_LIMIT) {
        let _ = std::fs::rename(&log, home.join("daemon.log.1"));
    }
    let out = std::fs::OpenOptions::new().create(true).append(true).open(&log)?;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(["daemon", "run"])
        .stdin(Stdio::null())
        .stdout(out.try_clone()?)
        .stderr(out);
    // SAFETY: setsid is async-signal-safe and touches no memory of ours; it runs
    // in the child between fork and exec.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command.spawn().map(|_| ())
}

/// Attach to the daemon (starting it if needed) and follow `squad`.
pub async fn attach(squad: &str) -> io::Result<(Client, Hello)> {
    ensure_running().await?;
    Client::connect(&socket(), &build_id(), squad).await
}

/// `equipe daemon stop`: `Ok(false)` when none was running.
pub async fn stop() -> io::Result<bool> {
    gateway::shutdown(&socket()).await
}
