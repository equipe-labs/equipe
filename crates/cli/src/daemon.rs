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

use crate::launchd;

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
    let idle = std::env::var("EQUIPE_TRIAL_IDLE_SECS").ok().and_then(|secs| secs.parse().ok()).map(Duration::from_secs);
    let leave = leave_for(&settled(&equipe_home()), &temp_roots(), idle);
    left_for(gateway::serve_leaving(config, &socket(), &build_id(), leave).await)
}

/// How long a daemon in a temporary home waits, with no screen and no member
/// at work, before it leaves.
pub const TRIAL_IDLE: Duration = Duration::from_secs(15 * 60);

/// Whether this daemon's home is a temporary folder — a test's or a check's —
/// whose daemon leaves on its own (see [`TRIAL_IDLE`]).
pub fn trial() -> bool {
    is_temporary(&settled(&equipe_home()), &temp_roots())
}

/// When the daemon serving `home` leaves on its own: one in a temporary home
/// once the home is gone, or after `idle` (else [`TRIAL_IDLE`]) alone; the
/// person's own, never. Started detached, a test's or a check's daemon
/// outlives them — sixteen once piled up, each keeping its squads. Either
/// leaves once hung, for the login service or the next screen to start it
/// again: the one of 0.103.30 answered nothing until restarted by hand.
fn leave_for(home: &Path, temps: &[PathBuf], idle: Option<Duration>) -> gateway::Leave {
    let hung = gateway::Leave { hung: Some(equipe_core::watchdog::HUNG_AFTER), ..gateway::Leave::NEVER };
    if !is_temporary(home, temps) {
        return hung;
    }
    gateway::Leave { home_gone: true, idle: Some(idle.unwrap_or(TRIAL_IDLE)), ..hung }
}

/// Where temporary folders are made: this process's, and the system's.
fn temp_roots() -> Vec<PathBuf> {
    let mut roots = vec![std::env::temp_dir()];
    roots.extend(["/tmp", "/var/tmp", "/var/folders", "/dev/shm"].map(PathBuf::from));
    roots
}

/// `home` as it really is, symbolic links followed, when it is there.
fn settled(home: &Path) -> PathBuf {
    home.canonicalize().unwrap_or_else(|_| home.to_owned())
}

fn is_temporary(home: &Path, temps: &[PathBuf]) -> bool {
    use std::path::Component;
    // `..` could lead back out of a temporary folder: such a home isn't taken for one.
    if home.components().any(|c| c == Component::ParentDir) {
        return false;
    }
    let home = unprivate(home);
    temps.iter().any(|root| home.starts_with(unprivate(root)))
}

/// On a Mac `/tmp` and `/var` are `/private/tmp` and `/private/var`.
fn unprivate(path: &Path) -> PathBuf {
    match path.strip_prefix("/private") {
        Ok(rest) if rest.starts_with("tmp") || rest.starts_with("var") => Path::new("/").join(rest),
        _ => path.to_owned(),
    }
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
///
/// With the login service installed, only the service starts it — in place,
/// or loaded afresh when launchd refuses this build — and never this process
/// on its own beside it: started from an SSH connection (the desktop app
/// restarts a Mac's daemon so), that one would run in the connection's
/// session, the login Keychain closed to it, every Claude member "signed
/// out". When the service can't, it is said why, and how to fix it.
pub async fn ensure_running() -> io::Result<bool> {
    let socket = socket();
    if gateway::is_running(&socket).await {
        return Ok(false);
    }
    match start_way(service::installed(), own_home()) {
        StartWay::Service => {
            let up = service::bring_up(launchd::Kick::Start, false).await?;
            note_reload(&up);
            Ok(true)
        }
        StartWay::Alone => {
            spawn_detached()?;
            if answers_within(&socket, Duration::from_secs(20)).await {
                Ok(true)
            } else {
                Err(io::Error::new(io::ErrorKind::TimedOut, equipe_core::t!("daemon.didnt_start", log = log_path().display())))
            }
        }
    }
}

/// Who starts the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartWay {
    /// Its login service, and nothing else.
    Service,
    /// This process, detached: only where there is no login service — or it
    /// is another home's (`EQUIPE_HOME` set elsewhere), which never touches it.
    Alone,
}

fn start_way(installed: bool, own_home: bool) -> StartWay {
    if installed && own_home { StartWay::Service } else { StartWay::Alone }
}

async fn answers_within(socket: &Path, wait: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + wait;
    while tokio::time::Instant::now() < deadline {
        if gateway::is_running(socket).await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// The daemon's log says when the service had to be loaded afresh, and why.
fn note_reload(up: &launchd::Up) {
    if let Some(line) = reload_line(up.reloaded) {
        note_in_log(line);
    }
}

fn reload_line(why: Option<launchd::Why>) -> Option<&'static str> {
    use launchd::Why;
    Some(match why? {
        Why::Changed => "equipe: the login service runs another executable now, so it was loaded afresh",
        Why::NotLoaded => "equipe: launchd didn't hold the login service, so it was loaded",
        Why::Elsewhere => "equipe: the login service moved from the user domain into the person's graphical session",
        Why::Refused => "equipe: launchd wouldn't run this build (exit 78, EX_CONFIG), so the login service was loaded afresh",
    })
}

/// How a restart went.
pub enum Restarted {
    /// Under its login service.
    Service(launchd::Up),
    /// On its own: there is no login service here (or this home isn't the
    /// person's own).
    Alone,
}

/// `equipe daemon restart`: the daemon stopped as `equipe daemon stop` asks
/// (its work journaled; `ended` when it had to be ended), and started again.
/// Under the login service that is `launchctl kickstart -k`, in place —
/// the plist loaded afresh only when it changed or launchd refuses the build
/// — and nothing else ever starts it.
pub async fn restart(ended: impl FnOnce(u32)) -> io::Result<Restarted> {
    let socket = socket();
    if start_way(service::installed(), own_home()) == StartWay::Alone {
        if let Ok(Stopped::Ended(pid)) = stop().await {
            ended(pid);
        }
        ensure_running().await?;
        return Ok(Restarted::Alone);
    }
    let exe = std::env::current_exe()?;
    let changed = service::changed(&exe, &log_path());
    // Before the running daemon is stopped: a file that can't be written leaves it running.
    if changed {
        service::write(&exe, &log_path())?;
    }
    let old = gateway::daemon_pid(&socket).await;
    if let Ok(Stopped::Ended(pid)) = stop().await {
        ended(pid);
    }
    // Gone before it is started again: `kickstart -k` would cut its last writes short.
    if let Some(pid) = old {
        gone_within(pid, STOP_WAIT).await;
    }
    let up = service::bring_up(launchd::Kick::Restart, changed).await?;
    note_reload(&up);
    Ok(Restarted::Service(up))
}

/// Wait for process `pid` to have exited, `wait` at most.
async fn gone_within(pid: u32, wait: Duration) {
    let deadline = tokio::time::Instant::now() + wait;
    // SAFETY: kill with signal 0 only asks; `pid` is a single process (> 1).
    while unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// What the person is told when the login service couldn't bring the
/// daemon up: what failed, that none was started on its own, and the command
/// for the Mac's own screen.
pub fn down_message(down: &launchd::Down, over_ssh: bool) -> String {
    use launchd::Down;
    let what = match down {
        Down::Bootstrap { tries, ran } => {
            let said = ran.text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_owned();
            equipe_core::t!("daemon.service_bootstrap_failed", tries = tries, said = format!("{}: {said}", ran.code))
        }
        Down::Refused { job } => equipe_core::t!("daemon.service_refused", code = job.last_exit.unwrap_or(launchd::EX_CONFIG)),
        Down::NoAnswer { job: Some(job) } => equipe_core::t!(
            "daemon.service_no_answer",
            state = if job.state.is_empty() { "?" } else { &job.state },
            exit = job.last_exit.map_or("-".to_owned(), |c| c.to_string())
        ),
        Down::NoAnswer { job: None } => equipe_core::t!("daemon.service_gone").to_owned(),
    };
    format!("{what} {}", not_alone(over_ssh))
}

/// That no daemon was started on its own beside the login service, and the
/// command that fixes the service — at the Mac's own screen, from SSH.
fn not_alone(over_ssh: bool) -> String {
    if over_ssh {
        equipe_core::t!("daemon.not_alone_ssh").to_owned()
    } else {
        equipe_core::t!("daemon.not_alone", log = log_path().display())
    }
}

/// Whether this process runs from an SSH connection.
pub fn over_ssh() -> bool {
    equipe_core::engine_auth::ssh_bound(&|name| std::env::var(name).ok())
}

/// How the daemon that answers runs, for `equipe daemon status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HowRuns {
    /// In a temporary home: leaves on its own.
    Trial,
    /// Its login service runs it.
    Service,
    /// On its own: started from an SSH connection (`ssh`), beside an
    /// installed login service (`beside`).
    Alone { ssh: bool, beside: bool },
}

/// The daemon as `equipe daemon status` tells it.
pub struct Whereabouts {
    pub how: HowRuns,
    /// As it tells it; `None` from a daemon older than `runs`.
    pub runs: Option<equipe_core::protocol::rpc::Runs>,
    /// The login service's job's last exit, when it wasn't a clean one.
    pub last_exit: Option<i32>,
}

pub async fn whereabouts() -> Whereabouts {
    let socket = socket();
    let runs = tokio::time::timeout(ANSWER_WAIT, gateway::runs(&socket)).await.ok().flatten();
    let pid = gateway::daemon_pid(&socket).await;
    let installed = service::installed() && own_home();
    let job = if installed { service::job() } else { None };
    Whereabouts {
        how: how_runs(trial(), installed, job.as_ref().and_then(|j| j.pid), pid, runs.as_ref()),
        runs,
        last_exit: job.and_then(|j| j.abnormal_exit()),
    }
}

/// How the daemon runs: the login service's when the process it runs is the
/// one answering (else as the daemon says), on its own otherwise.
fn how_runs(trial: bool, installed: bool, service_pid: Option<u32>, pid: Option<u32>, runs: Option<&equipe_core::protocol::rpc::Runs>) -> HowRuns {
    if trial {
        return HowRuns::Trial;
    }
    let by_service = match (service_pid, pid) {
        (Some(service), Some(pid)) => service == pid,
        _ => runs.map_or(installed, |r| r.service),
    };
    if installed && by_service {
        HowRuns::Service
    } else {
        HowRuns::Alone { ssh: runs.is_some_and(|r| r.ssh), beside: installed }
    }
}

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

    use crate::launchd::LABEL;

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
        // Nor launchctl's own words on standard error: a service is looked for
        // in both domains, and the one it isn't in says so ("Could not find
        // service…", "Boot-out failed: 3") though nothing went wrong; what did
        // is told in our words, from the status.
        Command::new(program).args(args).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
    }

    fn domains() -> [String; 2] {
        let (gui, user) = crate::launchd::domains(crate::launchd::uid());
        [gui, user]
    }

    /// What the service runs, and where it writes, as `program` and `log`
    /// would have it: the executable as the system resolves it, so a link
    /// to the same build (Homebrew's `bin/equipe`, an app's) is the same.
    fn runs(text: &str) -> Option<(PathBuf, Vec<String>, String)> {
        let unescape = |s: &str| s.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&");
        let strings = |within: &str| -> Vec<String> {
            within.split("<string>").skip(1).filter_map(|s| s.split_once("</string>")).map(|(s, _)| unescape(s)).collect()
        };
        let after = |key: &str| text.split_once(&format!("<key>{key}</key>")).map(|(_, rest)| rest);
        let array = after("ProgramArguments")?.split_once("</array>")?.0;
        let mut args = strings(array);
        if args.is_empty() {
            return None;
        }
        let exe = PathBuf::from(args.remove(0));
        let log = after("StandardOutPath").and_then(|rest| strings(rest).into_iter().next()).unwrap_or_default();
        Some((exe, args, log))
    }

    fn same_file(a: &Path, b: &Path) -> bool {
        a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
    }

    /// Whether the installed service would run something else than this
    /// build would install: another executable, other arguments, another
    /// log. Its PATH isn't asked: the one it was installed from stays.
    pub fn changed(exe: &Path, log: &Path) -> bool {
        if !cfg!(target_os = "macos") {
            return false;
        }
        let Ok(text) = std::fs::read_to_string(plist_path()) else { return true };
        is_changed(&text, &stable_exe(exe), log)
    }

    fn is_changed(text: &str, exe: &Path, log: &Path) -> bool {
        match runs(text) {
            Some((installed, args, logged)) => !(same_file(&installed, exe) && args == ["daemon", "run"] && logged == log.to_string_lossy()),
            None => true,
        }
    }

    /// Write the service's file for this build.
    pub fn write(exe: &Path, log: &Path) -> io::Result<()> {
        let exe = stable_exe(exe);
        let path = service_path(std::env::var("PATH").unwrap_or_default(), dirs::home_dir());
        let (file, text) = if cfg!(target_os = "macos") { (plist_path(), plist(&exe, &path, log)) } else { (unit_path(), unit(&exe, &path, log)) };
        std::fs::create_dir_all(file.parent().expect("the service's folder has a parent"))?;
        std::fs::write(&file, text)
    }

    /// Write the service and load it afresh, until its daemon answers. The
    /// running daemon must be stopped first.
    pub async fn install(exe: &Path, log: &Path) -> io::Result<()> {
        write(exe, log)?;
        if cfg!(target_os = "macos") {
            bring_up(crate::launchd::Kick::Restart, true).await.map(drop)
        } else if run("systemctl", &["--user", "daemon-reload"]) && run("systemctl", &["--user", "enable", "--now", "equipe"]) {
            Ok(())
        } else {
            Err(io::Error::other(equipe_core::t!("daemon.systemctl_failed")))
        }
    }

    /// Have the service bring the daemon up ([`crate::launchd::bring_up`]),
    /// `changed` when its file was just written; what failed, in words, when
    /// it couldn't. It never starts one on its own.
    pub async fn bring_up(kick: crate::launchd::Kick, changed: bool) -> io::Result<crate::launchd::Up> {
        let socket = super::socket();
        if !cfg!(target_os = "macos") {
            let verb = if kick == crate::launchd::Kick::Restart { "restart" } else { "start" };
            if run("systemctl", &["--user", verb, "equipe"]) && super::answers_within(&socket, std::time::Duration::from_secs(15)).await {
                let pid = equipe_core::gateway::daemon_pid(&socket).await.unwrap_or_default();
                return Ok(crate::launchd::Up { pid, reloaded: None, domain: "systemd".into() });
            }
            return Err(io::Error::other(format!("{} {}", equipe_core::t!("daemon.systemctl_failed"), super::not_alone(super::over_ssh()))));
        }
        let mut launchd = crate::launchd::Launchd { socket };
        let plist = plist_path().to_string_lossy().into_owned();
        crate::launchd::bring_up(&mut launchd, crate::launchd::uid(), LABEL, &plist, kick, changed)
            .await
            .map_err(|down| {
                // A restart the app left running over SSH has no screen: the log is where it is read.
                super::note_in_log(&format!("equipe: the login service didn't bring the daemon up, and none was started on its own: {down:?}"));
                io::Error::other(super::down_message(&down, super::over_ssh()))
            })
    }

    /// The service's job as its manager tells it, when it holds it.
    pub fn job() -> Option<crate::launchd::Job> {
        if cfg!(target_os = "macos") {
            let mut launchd = crate::launchd::Launchd { socket: super::socket() };
            return crate::launchd::find(&mut launchd, crate::launchd::uid(), LABEL).map(|(_, job)| job);
        }
        let out = Command::new("systemctl").args(["--user", "show", "-p", "MainPID", "--value", "equipe"]).stderr(std::process::Stdio::null()).output().ok()?;
        let pid: u32 = String::from_utf8_lossy(&out.stdout).trim().parse().ok().filter(|p| *p > 0)?;
        Some(crate::launchd::Job { state: "running".into(), pid: Some(pid), ..Default::default() })
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

        #[test]
        fn a_restart_reloads_the_plist_only_when_it_runs_something_else() {
            let log = Path::new("/h/.equipe/daemon.log");
            let installed = plist(Path::new("/x/a&b/equipe"), "/usr/bin", log);
            assert_eq!(runs(&installed), Some((PathBuf::from("/x/a&b/equipe"), vec!["daemon".to_owned(), "run".to_owned()], log.display().to_string())));
            assert!(!is_changed(&installed, Path::new("/x/a&b/equipe"), log), "the same: started again in place");
            assert!(!is_changed(&plist(Path::new("/x/a&b/equipe"), "/opt/homebrew/bin:/usr/bin", log), Path::new("/x/a&b/equipe"), log), "another PATH isn't another program");
            assert!(is_changed(&installed, Path::new("/y/equipe"), log), "another executable");
            assert!(is_changed(&installed, Path::new("/x/a&b/equipe"), Path::new("/elsewhere/daemon.log")), "another log");
            assert!(is_changed(&installed.replace("<string>run</string>", "<string>serve</string>"), Path::new("/x/a&b/equipe"), log), "other arguments");
            assert!(is_changed("not a plist", Path::new("/x/a&b/equipe"), log));
        }

        #[test]
        fn a_link_to_the_same_build_is_the_same_program() {
            let dir = std::env::temp_dir().join(format!("eq-svc-link-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let (exe, link) = (dir.join("equipe"), dir.join("bin-equipe"));
            std::fs::write(&exe, "").unwrap();
            std::os::unix::fs::symlink(&exe, &link).unwrap();
            let log = Path::new("/h/daemon.log");
            assert!(!is_changed(&plist(&link, "/usr/bin", log), &exe, log));
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::rpc::Runs;

    #[test]
    fn with_its_login_service_installed_the_daemon_never_starts_on_its_own() {
        assert_eq!(start_way(true, true), StartWay::Service, "only the service starts it: never one bound to an SSH connection beside it");
        assert_eq!(start_way(false, true), StartWay::Alone, "no login service here: on its own");
        assert_eq!(start_way(true, false), StartWay::Alone, "another home never touches the person's service");
        assert!(is_own_home(Path::new("/Users/a/.equipe"), Some(Path::new("/Users/a"))));
        assert!(!is_own_home(Path::new("/var/folders/x/T/tmp.V/"), Some(Path::new("/Users/a"))), "a home elsewhere starts alone, never through the person's service");
        assert!(!is_own_home(Path::new("/Users/a/.equipe"), None));
    }

    #[test]
    fn a_service_that_couldnt_bring_it_up_says_what_failed_and_what_to_run_at_the_macs_screen() {
        use launchd::{Down, Job, Ran};
        let refused = Down::Refused { job: Job { state: "spawn scheduled".into(), pid: None, last_exit: Some(78), lwcr: true } };
        let said = down_message(&refused, true);
        assert!(said.contains("78"), "{said}");
        assert!(said.contains("SSH") && said.contains("equipe daemon install") && said.contains("own screen"), "{said}");
        let bootstrap = Down::Bootstrap { tries: 8, ran: Ran { code: 5, text: "Bootstrap failed: 5: Input/output error\nTry re-running the command as root for richer errors.\n".into() } };
        let said = down_message(&bootstrap, false);
        assert!(said.contains("8") && said.contains("5: Bootstrap failed: 5: Input/output error") && !said.contains("root"), "{said}");
        assert!(said.contains("equipe daemon install") && said.contains("daemon.log"), "{said}");
        let silent = down_message(&Down::NoAnswer { job: Some(Job { state: "running".into(), pid: Some(7), last_exit: None, lwcr: false }) }, false);
        assert!(silent.contains("running"), "{silent}");
        assert!(!down_message(&Down::NoAnswer { job: None }, false).is_empty());
    }

    #[test]
    fn the_log_says_why_the_service_was_loaded_afresh() {
        use launchd::Why;
        assert_eq!(reload_line(None), None, "started again in place: nothing to say");
        for why in [Why::Changed, Why::NotLoaded, Why::Elsewhere, Why::Refused] {
            assert!(reload_line(Some(why)).is_some_and(|l| l.starts_with("equipe: ")), "{why:?}");
        }
        assert!(reload_line(Some(Why::Refused)).unwrap().contains("78"));
    }

    #[test]
    fn status_tells_a_daemon_its_service_runs_from_one_on_its_own_over_ssh() {
        let runs = |ssh, service| Runs { pid: 7, ssh, service, login_session: Some(!ssh), keychain: Some(!ssh) };
        assert_eq!(how_runs(true, true, Some(7), Some(7), None), HowRuns::Trial);
        assert_eq!(how_runs(false, true, Some(7), Some(7), Some(&runs(false, true))), HowRuns::Service);
        // The Mac mini after the update: the service installed, the daemon one an SSH restart started.
        assert_eq!(how_runs(false, true, None, Some(9), Some(&runs(true, false))), HowRuns::Alone { ssh: true, beside: true });
        assert_eq!(how_runs(false, true, Some(7), Some(9), Some(&runs(false, false))), HowRuns::Alone { ssh: false, beside: true }, "the pids decide");
        assert_eq!(how_runs(false, false, None, Some(9), Some(&runs(true, false))), HowRuns::Alone { ssh: true, beside: false });
        assert_eq!(how_runs(false, false, None, Some(9), None), HowRuns::Alone { ssh: false, beside: false });
        // A daemon older than `runs`, its service's pid unknown: as before, the service's.
        assert_eq!(how_runs(false, true, None, Some(9), None), HowRuns::Service);
    }

    #[test]
    fn a_daemon_beside_one_that_runs_leaves_cleanly_and_a_real_failure_stays_one() {
        assert!(left_for(Err(io::Error::new(io::ErrorKind::AddrInUse, "a daemon is already running"))).is_ok());
        assert!(left_for(Err(io::Error::other("disk"))).is_err());
        assert!(left_for(Ok(())).is_ok());
    }

    #[test]
    fn a_daemon_in_a_temporary_home_leaves_on_its_own_and_the_persons_never_does() {
        let temps = [PathBuf::from("/tmp"), PathBuf::from("/var/folders/hy/x/T/")];
        // Homes T-355's and T-362's checks left their daemons in.
        assert!(is_temporary(Path::new("/tmp/t355home"), &temps));
        assert!(is_temporary(Path::new("/private/tmp/t362home.38E6"), &temps), "/tmp is /private/tmp on a Mac");
        assert!(is_temporary(Path::new("/private/var/folders/hy/x/T/tmp.Pwnyv1UVsb"), &temps));
        assert!(!is_temporary(Path::new("/Users/a/.equipe"), &temps));
        assert!(!is_temporary(Path::new("/tmpfiles/home"), &temps), "a folder named like it isn't one");
        assert!(!is_temporary(Path::new("/tmp/../Users/a/.equipe"), &temps), "a way through /tmp back to the person's home isn't one");

        let hung = gateway::Leave { hung: Some(equipe_core::watchdog::HUNG_AFTER), ..gateway::Leave::NEVER };
        assert_eq!(leave_for(Path::new("/Users/a/.equipe"), &temps, None), hung, "the person's own leaves only when hung");
        assert_eq!(
            leave_for(Path::new("/Users/a/.equipe"), &temps, Some(Duration::from_secs(1))),
            hung,
            "no setting makes the person's own daemon leave"
        );
        let trial = leave_for(Path::new("/tmp/t355home"), &temps, None);
        assert!(trial.home_gone);
        assert_eq!(trial.idle, Some(TRIAL_IDLE));
        assert_eq!(trial.hung, Some(equipe_core::watchdog::HUNG_AFTER));
        assert_eq!(leave_for(Path::new("/tmp/t355home"), &temps, Some(Duration::from_secs(2))).idle, Some(Duration::from_secs(2)));
    }
}
