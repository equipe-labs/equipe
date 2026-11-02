//! Finding, starting and stopping the daemon from the command line.
//!
//! The daemon is this same executable running `equipe daemon run`, detached from
//! the terminal that started it: it gets a session of its own (`setsid`), so
//! closing that terminal never reaches it, and its output goes to
//! `~/.equipe/daemon.log`.

use std::io;
use std::path::{Path, PathBuf};
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
    if gateway::is_running(&socket()).await {
        equipe_core::daemon_log!("equipe: a daemon already answers here, so this one leaves");
        return Ok(());
    }
    let config = runtime::Config::for_user(VERSION).persistent();
    // Telegram, when set up, follows the squads like one more screen.
    tokio::spawn(crate::telegram::serve_forever(equipe_home(), socket(), build_id()));
    // Phones paired with this machine, through the relay.
    tokio::spawn(crate::remote::link::serve_forever(equipe_home(), socket(), build_id()));
    left_for(gateway::serve(config, &socket(), &build_id()).await)
}

/// A daemon that finds another already answering leaves cleanly: an exit the
/// login service would read as a crash and start again, every ten seconds,
/// beside the one that runs.
fn left_for(served: io::Result<()>) -> io::Result<()> {
    match served {
        Err(err) if err.kind() == io::ErrorKind::AddrInUse => {
            equipe_core::daemon_log!("equipe: a daemon already answers here, so this one leaves");
            Ok(())
        }
        other => other,
    }
}

/// Start a daemon in the background unless one is answering. `Ok(true)` when
/// this call started it.
pub async fn ensure_running() -> io::Result<bool> {
    let socket = socket();
    if gateway::is_running(&socket).await {
        return Ok(false);
    }
    let take = |way| match way {
        Way::Service => service::start(),
        Way::Reload => std::env::current_exe().is_ok_and(|exe| service::install(&exe, &log_path()).is_ok()),
        Way::Alone => spawn_detached().is_ok(),
    };
    match start_with(ways(service::installed() && own_home()), take, || gateway::is_running(&socket)).await {
        Some(way) => {
            if let Some(line) = way.log_line() {
                note_in_log(line);
            }
            Ok(true)
        }
        None => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            equipe_core::t!("daemon.didnt_start", log = log_path().display()),
        )),
    }
}

/// One way to start a daemon, tried while none answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Way {
    /// Ask the login service to run it, so it stays looked after.
    Service,
    /// Load the service afresh: launchd won't run an executable whose signature
    /// changed since the service was loaded — a build replaced in place exits 78
    /// (EX_CONFIG) on every try — until it is loaded again.
    Reload,
    /// Run it ourselves, detached, without the service.
    Alone,
}

impl Way {
    /// What the daemon's log says when this way, not the first, started it.
    fn log_line(self) -> Option<&'static str> {
        match self {
            Way::Service => None,
            Way::Reload => Some("equipe: the login service wouldn't start this build, so it was loaded afresh"),
            Way::Alone => Some(
                "equipe: the login service didn't start the daemon: it runs on its own until `equipe daemon install`",
            ),
        }
    }
}

/// The ways to try, in order, each with how long it waits for an answer.
/// Whether this is the person's own home (`~/.equipe`), the one the login
/// service runs. Another (`EQUIPE_HOME` set elsewhere: a test, a second
/// squad's trial) never touches the service: starting it there reloaded the
/// person's own daemon in the middle of its work and pointed its log at a
/// temporary folder.
pub fn own_home() -> bool {
    is_own_home(&equipe_home(), dirs::home_dir().as_deref())
}

fn is_own_home(home: &Path, user: Option<&Path>) -> bool {
    user.is_some_and(|u| home == u.join(".equipe"))
}

fn ways(installed: bool) -> &'static [(Way, Duration)] {
    const SERVICE: &[(Way, Duration)] = &[
        (Way::Service, Duration::from_secs(10)),
        (Way::Reload, Duration::from_secs(10)),
        (Way::Alone, Duration::from_secs(20)),
    ];
    const ALONE: &[(Way, Duration)] = &[(Way::Alone, Duration::from_secs(20))];
    if installed { SERVICE } else { ALONE }
}

/// Take each way in turn until a daemon answers; which one did, or `None`.
/// A way that can't even be taken gives way to the next at once — but a reload
/// that fails is still waited for: another start racing this one may have
/// loaded the service first (this bootstrap then fails, error 5), and the
/// daemon that brings beats a second one alone beside it.
async fn start_with<F: Future<Output = bool>>(
    ways: &[(Way, Duration)],
    mut take: impl FnMut(Way) -> bool,
    answering: impl Fn() -> F,
) -> Option<Way> {
    for &(way, wait) in ways {
        if !take(way) && way != Way::Reload {
            continue;
        }
        let deadline = tokio::time::Instant::now() + wait;
        let mut asked = tokio::time::Instant::now();
        while tokio::time::Instant::now() < deadline {
            if answering().await {
                return Some(way);
            }
            // A daemon just told to stop may still be winding down: the service, seeing
            // it alive, ignores the start, then leaves it stopped once it exits cleanly.
            // Ask again each second.
            if way == Way::Service && asked.elapsed() >= Duration::from_secs(1) {
                take(way);
                asked = tokio::time::Instant::now();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    None
}

fn note_in_log(line: &str) {
    use std::io::Write as _;
    if let Ok(mut log) = std::fs::OpenOptions::new().create(true).append(true).open(log_path()) {
        let _ = writeln!(log, "{line}");
    }
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

    /// The PATH the service runs with: the one it was installed from, and where
    /// the AI tools put themselves (Claude Code's installer: `~/.local/bin`,
    /// Homebrew's) — a daemon the app starts from Finder has only the system's,
    /// and could not find `claude` or `codex` the person installs afterwards.
    pub fn service_path(path: String, home: Option<PathBuf>) -> String {
        let usual = home.iter().map(|h| h.join(".local/bin")).chain(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
        let have: Vec<PathBuf> = std::env::split_paths(&path).collect();
        let missing: Vec<PathBuf> = usual.filter(|dir| !have.contains(dir)).collect();
        std::env::join_paths(missing.into_iter().chain(have)).map(|p| p.to_string_lossy().into_owned()).unwrap_or(path)
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
        let path = service_path(std::env::var("PATH").unwrap_or_default(), dirs::home_dir());
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
        fn the_service_finds_tools_installed_after_it_started() {
            let path = service_path("/usr/bin:/bin".into(), Some(PathBuf::from("/Users/me")));
            assert_eq!(path, "/Users/me/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin");
            // Already there: not added twice.
            assert_eq!(service_path(path.clone(), Some(PathBuf::from("/Users/me"))), path);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Ways as the real ones, with waits short enough for a test.
    const QUICK: &[(Way, Duration)] = &[
        (Way::Service, Duration::from_millis(200)),
        (Way::Reload, Duration::from_millis(200)),
        (Way::Alone, Duration::from_millis(200)),
    ];

    /// Start where only `works` brings a daemon up and only `takeable` ways can
    /// be taken: the ways taken, in order, and the one that started it.
    async fn start(works: Option<Way>, takeable: impl Fn(Way) -> bool) -> (Vec<Way>, Option<Way>) {
        let taken = RefCell::new(Vec::new());
        let up = RefCell::new(false);
        let started = start_with(
            QUICK,
            |way| {
                taken.borrow_mut().push(way);
                if Some(way) == works {
                    *up.borrow_mut() = true;
                }
                takeable(way)
            },
            || std::future::ready(*up.borrow()),
        )
        .await;
        let mut taken = taken.into_inner();
        taken.dedup();
        (taken, started)
    }

    #[tokio::test]
    async fn a_service_that_starts_the_daemon_is_all_it_takes() {
        assert_eq!(start(Some(Way::Service), |_| true).await, (vec![Way::Service], Some(Way::Service)));
    }

    #[tokio::test]
    async fn a_service_that_refuses_a_replaced_build_is_loaded_afresh() {
        // launchd takes the start and runs nothing: exit 78 on a re-signed executable.
        assert_eq!(start(Some(Way::Reload), |_| true).await, (vec![Way::Service, Way::Reload], Some(Way::Reload)));
    }

    #[tokio::test]
    async fn when_the_service_never_starts_it_the_daemon_runs_on_its_own() {
        assert_eq!(
            start(Some(Way::Alone), |way| way == Way::Alone).await,
            (vec![Way::Service, Way::Reload, Way::Alone], Some(Way::Alone))
        );
    }

    #[tokio::test]
    async fn a_daemon_nothing_starts_is_said_so() {
        assert_eq!(start(None, |_| true).await.1, None);
    }

    #[tokio::test]
    async fn a_reload_another_start_got_to_first_is_waited_for_not_doubled() {
        // This start's bootstrap fails, but the service the other loaded brings
        // the daemon up a moment later.
        let taken = RefCell::new(Vec::new());
        let looks = std::cell::Cell::new(0);
        let started = start_with(
            QUICK,
            |way| {
                taken.borrow_mut().push(way);
                way == Way::Alone
            },
            || {
                if taken.borrow().contains(&Way::Reload) {
                    looks.set(looks.get() + 1);
                }
                std::future::ready(looks.get() > 2)
            },
        )
        .await;
        assert_eq!(started, Some(Way::Reload));
        assert!(!taken.borrow().contains(&Way::Alone), "no second daemon beside it");
    }

    #[test]
    fn a_daemon_beside_one_that_runs_leaves_cleanly_and_a_real_failure_stays_one() {
        assert!(left_for(Err(io::Error::new(io::ErrorKind::AddrInUse, "a daemon is already running"))).is_ok());
        assert!(left_for(Err(io::Error::other("disk"))).is_err());
        assert!(left_for(Ok(())).is_ok());
    }

    #[test]
    fn without_the_service_it_runs_on_its_own_and_the_log_says_how_it_started() {
        assert_eq!(ways(false).iter().map(|w| w.0).collect::<Vec<_>>(), [Way::Alone]);
        assert!(is_own_home(Path::new("/Users/a/.equipe"), Some(Path::new("/Users/a"))));
        assert!(!is_own_home(Path::new("/var/folders/x/T/tmp.V/"), Some(Path::new("/Users/a"))), "a home elsewhere starts alone, never through the person's service");
        assert!(!is_own_home(Path::new("/Users/a/.equipe"), None));
        assert_eq!(ways(true).iter().map(|w| w.0).collect::<Vec<_>>(), [Way::Service, Way::Reload, Way::Alone]);
        assert_eq!(Way::Service.log_line(), None);
        assert!(Way::Reload.log_line().is_some() && Way::Alone.log_line().is_some());
    }
}
