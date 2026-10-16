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
    // Telegram, when set up, follows the squads like one more screen.
    tokio::spawn(crate::telegram::serve_forever(equipe_home(), socket(), build_id()));
    // Phones paired with this machine, through the relay.
    tokio::spawn(crate::remote::link::serve_forever(equipe_home(), socket(), build_id()));
    gateway::serve(config, &socket(), &build_id()).await
}

/// Start a daemon in the background unless one is answering. `Ok(true)` when
/// this call started it.
pub async fn ensure_running() -> io::Result<bool> {
    let socket = socket();
    if gateway::is_running(&socket).await {
        return Ok(false);
    }
    // Installed as a service, the service starts it, so it stays looked after.
    let service = service::installed() && service::start();
    if !service {
        spawn_detached()?;
    }
    for i in 0..400 {
        if gateway::is_running(&socket).await {
            return Ok(true);
        }
        // A daemon just told to stop may still be winding down: the service, seeing
        // it alive, ignores the start, then leaves it stopped once it exits cleanly.
        // Ask again each second until one answers.
        if service && i % 20 == 19 {
            service::start();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        equipe_core::t!("daemon.didnt_start", log = log_path().display()),
    ))
}

pub fn log_path() -> PathBuf {
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
    Client::connect_in(&socket(), &build_id(), squad, Some(crate::i18n::lang())).await
}

/// How long `equipe daemon status` waits for the daemon to answer.
pub const ANSWER_WAIT: Duration = Duration::from_secs(5);
/// How long a stop waits for the daemon to put its turns down before it is ended.
const STOP_WAIT: Duration = Duration::from_secs(15);

/// What the daemon says when asked what it is doing.
pub enum Answer {
    /// None runs: nothing listens at the socket.
    NotRunning,
    /// It answered — maybe that its core is stuck (`stalled`).
    Busy(equipe_core::protocol::rpc::Busy),
    /// It takes connections but answered nothing in [`ANSWER_WAIT`].
    Silent,
}

/// Ask the daemon what it is doing, never waiting longer than [`ANSWER_WAIT`].
pub async fn ask() -> Answer {
    let socket = socket();
    if !tokio::time::timeout(ANSWER_WAIT, gateway::is_running(&socket)).await.unwrap_or(false) {
        return Answer::NotRunning;
    }
    match tokio::time::timeout(ANSWER_WAIT, gateway::busy(&socket)).await {
        Ok(Ok(Some(busy))) => Answer::Busy(busy),
        // Older than `busy`: it answers, with nothing to say.
        Ok(Ok(None)) => Answer::Busy(Default::default()),
        Ok(Err(_)) | Err(_) => Answer::Silent,
    }
}

/// How `equipe daemon stop` went.
pub enum Stopped {
    None,
    /// It stopped when asked.
    Asked,
    /// It didn't, and was ended: this process.
    Ended(u32),
}

/// `equipe daemon stop`: asked to stop, a daemon puts its turns down and
/// exits; one that doesn't within [`STOP_WAIT`] — it no longer answers — is
/// ended (TERM, then KILL). Under the login service it then starts again.
pub async fn stop() -> io::Result<Stopped> {
    let socket = socket();
    if !gateway::is_running(&socket).await {
        return Ok(Stopped::None);
    }
    let pid = gateway::daemon_pid(&socket).await;
    match tokio::time::timeout(STOP_WAIT, gateway::shutdown(&socket)).await {
        Ok(Ok(true)) => return Ok(Stopped::Asked),
        Ok(Ok(false)) => return Ok(Stopped::None),
        Ok(Err(_)) | Err(_) => {}
    }
    let Some(pid) = pid else {
        return Err(io::Error::new(io::ErrorKind::TimedOut, equipe_core::t!("daemon.no_pid")));
    };
    end(pid).await;
    Ok(Stopped::Ended(pid))
}

/// End process `pid`: TERM, and KILL when it is still there a few seconds on.
async fn end(pid: u32) {
    let pid = pid as libc::pid_t;
    // SAFETY: kill has no memory preconditions; `pid` is a single process (> 1).
    let gone = || unsafe { libc::kill(pid, 0) } != 0;
    unsafe { libc::kill(pid, libc::SIGTERM) };
    for _ in 0..50 {
        if gone() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    unsafe { libc::kill(pid, libc::SIGKILL) };
    for _ in 0..20 {
        if gone() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The daemon as a login service — launchd on macOS, systemd on Linux — so it
/// starts when the person logs in, comes back if it crashes, and outlives the
/// terminal or SSH session it was started from. `equipe daemon stop` exits
/// cleanly, which the service leaves stopped.
pub mod service {
    use std::io;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    const LABEL: &str = "im.equipe.daemon";

    fn home() -> PathBuf {
        dirs::home_dir().unwrap_or_default()
    }

    fn plist_path() -> PathBuf {
        home().join("Library/LaunchAgents").join(format!("{LABEL}.plist"))
    }

    fn unit_path() -> PathBuf {
        home().join(".config/systemd/user/equipe.service")
    }

    pub fn installed() -> bool {
        if cfg!(target_os = "macos") { plist_path().is_file() } else { unit_path().is_file() }
    }

    /// The executable the service runs: one that survives updates — Homebrew's
    /// `bin/equipe` rather than the versioned Cellar path behind it.
    pub fn stable_exe(exe: &Path) -> PathBuf {
        let text = exe.to_string_lossy();
        match text.find("/Cellar/equipe/") {
            Some(at) => PathBuf::from(format!("{}/bin/equipe", &text[..at])),
            None => exe.to_owned(),
        }
    }

    fn escape(text: &str) -> String {
        text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    pub fn plist(exe: &Path, path: &str, log: &Path) -> String {
        let (exe, path, log) = (escape(&exe.to_string_lossy()), escape(path), escape(&log.to_string_lossy()));
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array><string>{exe}</string><string>daemon</string><string>run</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>EnvironmentVariables</key><dict><key>PATH</key><string>{path}</string></dict>
  <key>StandardOutPath</key><string>{log}</string>
  <key>StandardErrorPath</key><string>{log}</string>
</dict>
</plist>
"#
        )
    }

    pub fn unit(exe: &Path, path: &str, log: &Path) -> String {
        format!(
            "[Unit]\nDescription=equipe — the squad's daemon\n\n[Service]\nExecStart={} daemon run\nEnvironment=PATH={path}\nRestart=on-failure\nStandardOutput=append:{}\nStandardError=append:{}\n\n[Install]\nWantedBy=default.target\n",
            exe.display(),
            log.display(),
            log.display()
        )
    }

    fn run(program: &str, args: &[&str]) -> bool {
        // Nothing on our standard output: `equipe attach --stdio` carries the protocol there.
        Command::new(program).args(args).stdout(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
    }

    fn domains() -> [String; 2] {
        // SAFETY: getuid has no preconditions and cannot fail.
        let uid = unsafe { libc::getuid() };
        [format!("gui/{uid}"), format!("user/{uid}")]
    }

    /// Write the service and start it. The running daemon must be stopped first.
    pub fn install(exe: &Path, log: &Path) -> io::Result<()> {
        let exe = stable_exe(exe);
        let path = std::env::var("PATH").unwrap_or_default();
        if cfg!(target_os = "macos") {
            let file = plist_path();
            std::fs::create_dir_all(file.parent().expect("LaunchAgents has a parent"))?;
            std::fs::write(&file, plist(&exe, &path, log))?;
            let file = file.to_string_lossy().into_owned();
            // The person's graphical session when there is one, else their user domain.
            for domain in domains() {
                let _ = run("launchctl", &["bootout", &format!("{domain}/{LABEL}")]);
                if run("launchctl", &["bootstrap", &domain, &file]) {
                    return Ok(());
                }
            }
            Err(io::Error::other(equipe_core::t!("daemon.launchctl_failed")))
        } else {
            let file = unit_path();
            std::fs::create_dir_all(file.parent().expect("systemd/user has a parent"))?;
            std::fs::write(&file, unit(&exe, &path, log))?;
            if run("systemctl", &["--user", "daemon-reload"]) && run("systemctl", &["--user", "enable", "--now", "equipe"]) {
                Ok(())
            } else {
                Err(io::Error::other(equipe_core::t!("daemon.systemctl_failed")))
            }
        }
    }

    /// Stop the service and remove it. `Ok(false)` when there was none.
    pub fn uninstall() -> io::Result<bool> {
        if !installed() {
            return Ok(false);
        }
        if cfg!(target_os = "macos") {
            for domain in domains() {
                let _ = run("launchctl", &["bootout", &format!("{domain}/{LABEL}")]);
            }
            std::fs::remove_file(plist_path())?;
        } else {
            let _ = run("systemctl", &["--user", "disable", "--now", "equipe"]);
            std::fs::remove_file(unit_path())?;
            let _ = run("systemctl", &["--user", "daemon-reload"]);
        }
        Ok(true)
    }

    /// Start the installed service's daemon.
    pub fn start() -> bool {
        if cfg!(target_os = "macos") {
            domains().iter().any(|domain| run("launchctl", &["kickstart", &format!("{domain}/{LABEL}")]))
        } else {
            run("systemctl", &["--user", "start", "equipe"])
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_service_runs_a_path_that_survives_updates() {
            assert_eq!(stable_exe(Path::new("/opt/homebrew/Cellar/equipe/0.2.0/bin/equipe")), Path::new("/opt/homebrew/bin/equipe"));
            assert_eq!(stable_exe(Path::new("/Users/me/.local/bin/equipe")), Path::new("/Users/me/.local/bin/equipe"));
        }

        #[test]
        fn launchd_restarts_a_crash_but_not_a_stop() {
            let plist = plist(Path::new("/x/equipe"), "/usr/bin:/a&b", Path::new("/h/daemon.log"));
            assert!(plist.contains("<string>/x/equipe</string><string>daemon</string><string>run</string>"));
            assert!(plist.contains("<key>SuccessfulExit</key><false/>"), "a clean stop stays stopped");
            assert!(plist.contains("/usr/bin:/a&amp;b"), "escaped");
            let unit = unit(Path::new("/x/equipe"), "/usr/bin", Path::new("/h/daemon.log"));
            assert!(unit.contains("ExecStart=/x/equipe daemon run") && unit.contains("Restart=on-failure"));
        }
    }
}
