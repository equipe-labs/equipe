//! The daemon's login service on a Mac, through `launchctl`: started again
//! in place, loaded afresh only when it has to be, and watched until the
//! daemon it runs answers at its socket.
//!
//! In place is `launchctl kickstart -k`: the job stays loaded, and launchd
//! ends its process and starts it again — from an SSH connection too, for
//! the same user, and still in the person's graphical session, where the
//! login Keychain is open. Loading afresh (`bootout`, then `bootstrap`) is
//! for a service whose plist changed, one launchd doesn't hold, one in the
//! wrong domain, and a build launchd refuses: an executable replaced or
//! re-signed since the job was loaded exits 78 (EX_CONFIG) on every start,
//! "needs LWCR update", until the job is loaded again. `bootout` returns
//! before the job is gone — `launchctl print` says `state = SIGTERMed` for
//! as long as the process takes to end — and a `bootstrap` meanwhile fails
//! ("5: Input/output error"), so the job's going is waited for, and a
//! bootstrap that fails so is tried again, a little later each time.
//!
//! The decisions are here, the commands behind [`Host`], so a test plays
//! launchd.

use std::time::Duration;

/// The service's label.
pub const LABEL: &str = equipe_core::engine_auth::SERVICE;

/// How a program says its configuration is wrong (sysexits.h): what launchd
/// records for a build it refuses to run.
pub const EX_CONFIG: i32 = 78;

/// How often the job is looked at while waiting.
const POLL: Duration = Duration::from_millis(250);
/// How long a booted-out job may take to go (a daemon puts its work down first).
const GONE_WAIT: Duration = Duration::from_secs(25);
/// How long a started daemon may take to answer at its socket.
const UP_WAIT: Duration = Duration::from_secs(15);
/// How many times a bootstrap is tried while launchd says it can't yet.
const BOOTSTRAP_TRIES: u32 = 8;
/// How many times the job is loaded afresh before giving up: a refused build
/// is usually run on the first.
const RELOADS: u32 = 2;

/// What `launchctl` did: its exit, and what it said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ran {
    pub code: i32,
    pub text: String,
}

impl Ran {
    pub fn ok(&self) -> bool {
        self.code == 0
    }

    /// launchd can't do it yet, but will: the job is still going, or another
    /// start is loading it (5 EIO, 16 EBUSY, 17 EEXIST, 37 EALREADY).
    fn not_yet(&self) -> bool {
        matches!(self.code, 5 | 16 | 17 | 37) || self.text.contains("already") || self.text.contains("in progress")
    }
}

/// What the flow acts through: `launchctl`, the clock, and the daemon's socket.
pub(crate) trait Host {
    fn launchctl(&mut self, args: &[&str]) -> Ran;
    async fn pause(&mut self, wait: Duration);
    /// The process answering at the daemon's socket, if one does.
    async fn answering(&mut self) -> Option<u32>;
}

/// The job, as `launchctl print` tells it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Job {
    /// `running`, `not running`, `spawn scheduled`, `SIGTERMed`…
    pub state: String,
    pub pid: Option<u32>,
    /// How its process last exited; `None` when it never has.
    pub last_exit: Option<i32>,
    /// launchd marks the executable's signature as changed since it was loaded.
    pub lwcr: bool,
}

impl Job {
    /// The job from `launchctl print <domain>/<label>`: its own lines are
    /// indented once; deeper ones belong to its sections.
    pub fn parse(text: &str) -> Job {
        let mut job = Job { lwcr: text.to_ascii_lowercase().contains("needs lwcr update"), ..Job::default() };
        for line in text.lines() {
            let Some(own) = line.strip_prefix('\t').filter(|rest| !rest.starts_with('\t')) else {
                continue;
            };
            let Some((key, value)) = own.split_once(" = ") else {
                continue;
            };
            match key.trim() {
                "state" => job.state = value.trim().to_owned(),
                "pid" => job.pid = value.trim().parse().ok(),
                // `78: EX_CONFIG`, `0`, `(never exited)`.
                "last exit code" => job.last_exit = value.split(':').next().and_then(|code| code.trim().parse().ok()),
                _ => {}
            }
        }
        job
    }

    pub fn running(&self) -> bool {
        self.state == "running" && self.pid.is_some()
    }

    /// launchd won't run this build until the job is loaded again.
    pub fn refuses(&self) -> bool {
        !self.running() && (self.last_exit == Some(EX_CONFIG) || self.lwcr)
    }

    /// Booted out, its process not yet ended.
    fn going(&self) -> bool {
        self.state.starts_with("SIG") || self.state == "exiting"
    }

    /// Its last exit, when it wasn't a clean one.
    pub fn abnormal_exit(&self) -> Option<i32> {
        self.last_exit.filter(|code| *code != 0)
    }
}

/// Whether the job's start is a first start or a restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kick {
    /// Start it when it isn't running; a daemon on its way up is waited for.
    Start,
    /// End the one running, if any, and start it again (`kickstart -k`).
    Restart,
}

/// Why the job was loaded afresh rather than started again in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// Its plist changed: another executable, or other arguments.
    Changed,
    /// launchd didn't hold it.
    NotLoaded,
    /// It was in the user domain while the person's graphical session is there.
    Elsewhere,
    /// launchd refused the build (exit 78, EX_CONFIG).
    Refused,
}

/// The daemon up under its login service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Up {
    pub pid: u32,
    /// `None`: started in place.
    pub reloaded: Option<Why>,
    /// `gui/<uid>`, or `user/<uid>` when no one is logged in at the screen.
    pub domain: String,
}

/// Why the login service couldn't bring the daemon up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Down {
    /// `launchctl bootstrap` failed, `tries` times; its last answer.
    Bootstrap { tries: u32, ran: Ran },
    /// launchd holds the job but won't run this build, loaded afresh too.
    Refused { job: Job },
    /// The daemon didn't answer at its socket; the job as last seen.
    NoAnswer { job: Option<Job> },
}

/// The person's graphical session's domain, and their user domain.
pub fn domains(uid: u32) -> (String, String) {
    (format!("gui/{uid}"), format!("user/{uid}"))
}

/// Whether someone is logged in at the Mac's screen as `uid`: the graphical
/// session's domain is there.
pub(crate) fn has_gui(host: &mut impl Host, uid: u32) -> bool {
    host.launchctl(&["print", &domains(uid).0]).ok()
}

/// The job at `target` (`<domain>/<label>`), when launchd holds it.
pub(crate) fn print(host: &mut impl Host, target: &str) -> Option<Job> {
    let ran = host.launchctl(&["print", target]);
    ran.ok().then(|| Job::parse(&ran.text))
}

/// The job wherever it is loaded: the graphical session's domain first.
pub(crate) fn find(host: &mut impl Host, uid: u32, label: &str) -> Option<(String, Job)> {
    let (gui, user) = domains(uid);
    [gui, user].into_iter().find_map(|domain| print(host, &format!("{domain}/{label}")).map(|job| (domain, job)))
}

/// Bring the daemon up under its login service, its plist at `plist`
/// (`changed` when just rewritten): in place when it can be, loaded afresh
/// when it must be. Never anything but launchd starts it.
pub(crate) async fn bring_up(host: &mut impl Host, uid: u32, label: &str, plist: &str, kick: Kick, changed: bool) -> Result<Up, Down> {
    let (gui, user) = domains(uid);
    let domain = if has_gui(host, uid) { gui } else { user.clone() };
    let target = format!("{domain}/{label}");
    let mut why = changed.then_some(Why::Changed);
    // One loaded in the user domain gives way to the graphical session's: only there is the Keychain open.
    if domain != user {
        let there = format!("{user}/{label}");
        if print(host, &there).is_some() {
            host.launchctl(&["bootout", &there]);
            wait_gone(host, &there).await;
            why = why.or(Some(Why::Elsewhere));
        }
    }
    if why.is_none() {
        why = match print(host, &target) {
            None => Some(Why::NotLoaded),
            Some(job) if job.refuses() => Some(Why::Refused),
            Some(job) => {
                let kicked = if kick == Kick::Start && job.running() {
                    true
                } else {
                    let mut args = vec!["kickstart"];
                    if kick == Kick::Restart {
                        args.push("-k");
                    }
                    args.push(&target);
                    host.launchctl(&args).ok()
                };
                if !kicked {
                    Some(Why::NotLoaded)
                } else {
                    match wait_up(host, &target).await {
                        Waited::Up(pid) => return Ok(Up { pid, reloaded: None, domain }),
                        Waited::Refused(_) => Some(Why::Refused),
                        Waited::Gone => Some(Why::NotLoaded),
                        // It runs and doesn't answer (or another holds the socket):
                        // loading it again would end it, and answer no better.
                        Waited::Silent(job) => return Err(Down::NoAnswer { job }),
                    }
                }
            }
        };
    }
    let why = why.unwrap_or(Why::NotLoaded);
    let mut down = Down::NoAnswer { job: None };
    for _ in 0..RELOADS {
        reload(host, &domain, &target, plist).await?;
        match wait_up(host, &target).await {
            Waited::Up(pid) => return Ok(Up { pid, reloaded: Some(why), domain }),
            Waited::Refused(job) => down = Down::Refused { job },
            Waited::Gone => down = Down::NoAnswer { job: None },
            // Loading it again wouldn't make it answer.
            Waited::Silent(job) => return Err(Down::NoAnswer { job }),
        }
    }
    Err(down)
}

/// Load the job afresh: out, gone, and in again, tried again while launchd
/// says it can't yet.
async fn reload(host: &mut impl Host, domain: &str, target: &str, plist: &str) -> Result<(), Down> {
    if print(host, target).is_some() {
        host.launchctl(&["bootout", target]);
    }
    wait_gone(host, target).await;
    let mut pause = POLL;
    let mut tries = 0;
    loop {
        tries += 1;
        let ran = host.launchctl(&["bootstrap", domain, plist]);
        if ran.ok() {
            return Ok(());
        }
        if !ran.not_yet() || tries >= BOOTSTRAP_TRIES {
            return Err(Down::Bootstrap { tries, ran });
        }
        // Another start loaded it first: loaded is all this wanted.
        if print(host, target).is_some_and(|job| !job.going()) {
            return Ok(());
        }
        host.pause(pause).await;
        pause = (pause * 2).min(Duration::from_secs(2));
    }
}

/// Wait for a booted-out job to be gone.
async fn wait_gone(host: &mut impl Host, target: &str) {
    let mut waited = Duration::ZERO;
    while waited < GONE_WAIT && print(host, target).is_some() {
        host.pause(POLL).await;
        waited += POLL;
    }
}

enum Waited {
    /// Running, its process the one answering at the socket.
    Up(u32),
    Refused(Job),
    /// launchd no longer holds it.
    Gone,
    /// Never answered; the job as last seen.
    Silent(Option<Job>),
}

/// Wait for the job's daemon to answer at its socket.
async fn wait_up(host: &mut impl Host, target: &str) -> Waited {
    let mut waited = Duration::ZERO;
    loop {
        let Some(job) = print(host, target) else {
            return Waited::Gone;
        };
        if job.running() && host.answering().await == job.pid {
            return Waited::Up(job.pid.unwrap_or_default());
        }
        if job.refuses() {
            return Waited::Refused(job);
        }
        if waited >= UP_WAIT {
            return Waited::Silent(Some(job));
        }
        host.pause(POLL).await;
        waited += POLL;
    }
}

/// The real thing: `launchctl` itself, the clock, the daemon's socket.
pub(crate) struct Launchd {
    pub socket: std::path::PathBuf,
}

impl Host for Launchd {
    fn launchctl(&mut self, args: &[&str]) -> Ran {
        // Captured, never ours: `equipe attach --stdio` carries the protocol on our standard output.
        match std::process::Command::new("launchctl").args(args).stdin(std::process::Stdio::null()).output() {
            Ok(out) => Ran {
                code: out.status.code().unwrap_or(-1),
                text: format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
            },
            Err(err) => Ran { code: -1, text: err.to_string() },
        }
    }

    async fn pause(&mut self, wait: Duration) {
        tokio::time::sleep(wait).await;
    }

    async fn answering(&mut self) -> Option<u32> {
        equipe_core::gateway::daemon_pid(&self.socket).await
    }
}

/// The user this runs as.
pub fn uid() -> u32 {
    #[cfg(unix)]
    {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
    }
    #[cfg(windows)]
    { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// launchd, played: one job, in one domain, and what it does to each command.
    struct Fake {
        /// Someone is logged in at the screen.
        gui: bool,
        /// Where the job is loaded, if anywhere.
        loaded: Option<String>,
        state: &'static str,
        pid: Option<u32>,
        last_exit: Option<i32>,
        lwcr: bool,
        /// The build is refused until loaded afresh this many times.
        refused_until_loads: u32,
        loads: u32,
        /// After a bootout, the job takes this long to go.
        teardown: Duration,
        gone_at: Option<Duration>,
        /// Bootstraps that fail with 5 even once the job is gone.
        flaky_bootstraps: u32,
        /// Bootstraps tried while the job was still loaded (or going).
        early_bootstraps: u32,
        /// The daemon answers at its socket once running.
        answers: bool,
        clock: Duration,
        next_pid: u32,
        calls: Vec<String>,
    }

    impl Fake {
        fn loaded_and_running() -> Self {
            Fake {
                gui: true,
                loaded: Some("gui/501".into()),
                state: "running",
                pid: Some(100),
                last_exit: None,
                lwcr: false,
                refused_until_loads: 0,
                loads: 0,
                teardown: Duration::ZERO,
                gone_at: None,
                flaky_bootstraps: 0,
                early_bootstraps: 0,
                answers: true,
                clock: Duration::ZERO,
                next_pid: 200,
                calls: Vec::new(),
            }
        }

        fn spawn(&mut self) {
            if self.loads < self.refused_until_loads {
                (self.state, self.pid, self.last_exit) = ("spawn scheduled", None, Some(EX_CONFIG));
            } else {
                (self.state, self.pid) = ("running", Some(self.next_pid));
                self.next_pid += 1;
            }
        }

        fn settle(&mut self) {
            if self.gone_at.is_some_and(|at| self.clock >= at) {
                (self.loaded, self.gone_at, self.state, self.pid) = (None, None, "", None);
            }
        }

        fn ran(code: i32, text: &str) -> Ran {
            Ran { code, text: text.into() }
        }

        fn did(&self, what: &str) -> usize {
            self.calls.iter().filter(|c| c.starts_with(what)).count()
        }
    }

    impl Host for Fake {
        fn launchctl(&mut self, args: &[&str]) -> Ran {
            self.settle();
            self.calls.push(args.join(" "));
            let job = |target: &str| target.rsplit_once('/').map(|(domain, _)| domain.to_owned());
            match args {
                ["print", "gui/501"] => Self::ran(if self.gui { 0 } else { 125 }, "gui/501 = {"),
                ["print", target] => match &self.loaded {
                    Some(at) if job(target).as_ref() == Some(at) => {
                        let exit = self.last_exit.map_or("(never exited)".to_owned(), |c| format!("{c}: whatever"));
                        let pid = self.pid.map(|p| format!("\tpid = {p}\n")).unwrap_or_default();
                        let lwcr = if self.lwcr { "\tproperties = runatload | needs LWCR update\n" } else { "" };
                        Self::ran(0, &format!("{target} = {{\n\tstate = {}\n{pid}\tlast exit code = {exit}\n{lwcr}\tresource coalition = {{\n\t\tstate = active\n\t}}\n}}\n", self.state))
                    }
                    _ => Self::ran(113, "Could not find service"),
                },
                ["kickstart", rest @ ..] => {
                    let restart = rest.contains(&"-k");
                    if self.loaded.is_none() || self.gone_at.is_some() {
                        return Self::ran(113, "Could not find service");
                    }
                    if restart || !self.state.starts_with("running") {
                        self.spawn();
                    }
                    Self::ran(0, "")
                }
                ["bootout", target] => {
                    if self.loaded.as_deref() != job(target).as_deref() {
                        return Self::ran(3, "Boot-out failed: 3: No such process");
                    }
                    self.state = "SIGTERMed";
                    self.gone_at = Some(self.clock + self.teardown);
                    self.settle();
                    Self::ran(0, "")
                }
                ["bootstrap", domain, _plist] => {
                    self.early_bootstraps += u32::from(self.loaded.is_some());
                    if self.loaded.is_some() || self.flaky_bootstraps > 0 {
                        self.flaky_bootstraps = self.flaky_bootstraps.saturating_sub(u32::from(self.loaded.is_none()));
                        return Self::ran(5, "Bootstrap failed: 5: Input/output error");
                    }
                    (self.loaded, self.last_exit, self.lwcr) = (Some((*domain).to_owned()), None, false);
                    self.loads += 1;
                    self.spawn();
                    Self::ran(0, "")
                }
                _ => Self::ran(64, "unexpected"),
            }
        }

        async fn pause(&mut self, wait: Duration) {
            self.clock += wait;
        }

        async fn answering(&mut self) -> Option<u32> {
            self.settle();
            (self.answers && self.state == "running").then_some(self.pid).flatten()
        }
    }

    const PLIST: &str = "/Users/a/Library/LaunchAgents/im.equipe.daemon.plist";

    async fn up(fake: &mut Fake, kick: Kick, changed: bool) -> Result<Up, Down> {
        bring_up(fake, 501, LABEL, PLIST, kick, changed).await
    }

    #[tokio::test]
    async fn a_restart_starts_the_service_again_in_place() {
        let mut fake = Fake::loaded_and_running();
        let up = up(&mut fake, Kick::Restart, false).await.unwrap();
        assert_eq!(up, Up { pid: 200, reloaded: None, domain: "gui/501".into() });
        assert!(fake.calls.contains(&"kickstart -k gui/501/im.equipe.daemon".to_owned()), "{:?}", fake.calls);
        assert_eq!(fake.did("bootout") + fake.did("bootstrap"), 0, "no bootout, no bootstrap: {:?}", fake.calls);
    }

    #[tokio::test]
    async fn a_start_waits_for_a_daemon_already_on_its_way_up_and_ends_none() {
        let mut fake = Fake::loaded_and_running();
        fake.answers = false;
        let pending = up(&mut fake, Kick::Start, false).await;
        assert!(matches!(pending, Err(Down::NoAnswer { .. })), "it never answered: said so, not doubled");
        assert_eq!(fake.did("kickstart"), 0, "running already: left to come up");
        assert_eq!(fake.did("bootstrap"), 0, "a daemon that doesn't answer isn't loaded again: {:?}", fake.calls);
    }

    #[tokio::test]
    async fn a_changed_plist_is_loaded_afresh_once_the_old_job_is_gone_through_launchds_error_5() {
        let mut fake = Fake::loaded_and_running();
        // As on this Mac: the old job stays `SIGTERMed` for seconds after bootout returns.
        fake.teardown = Duration::from_secs(3);
        fake.flaky_bootstraps = 2;
        let up = up(&mut fake, Kick::Restart, true).await.unwrap();
        assert_eq!(up.reloaded, Some(Why::Changed));
        assert_eq!(fake.did("kickstart"), 0);
        assert_eq!(fake.did("bootout gui/501/im.equipe.daemon"), 1);
        assert_eq!(fake.early_bootstraps, 0, "the old job is waited for: {:?}", fake.calls);
        assert_eq!(fake.did("bootstrap gui/501"), 3, "error 5 twice, then loaded: {:?}", fake.calls);
        assert!(fake.clock >= Duration::from_secs(3));
    }

    #[tokio::test]
    async fn a_bootstrap_that_never_takes_says_how_it_failed() {
        let mut fake = Fake::loaded_and_running();
        fake.flaky_bootstraps = 100;
        let down = up(&mut fake, Kick::Restart, true).await.unwrap_err();
        let Down::Bootstrap { tries, ran } = down else { panic!("{down:?}") };
        assert_eq!(tries, BOOTSTRAP_TRIES);
        assert_eq!(ran.code, 5);
        assert!(fake.clock < Duration::from_secs(30), "tried a few times, not for good: {:?}", fake.clock);
    }

    #[tokio::test]
    async fn a_bootstrap_launchd_refuses_outright_isnt_tried_again() {
        let mut fake = Fake::loaded_and_running();
        fake.loaded = None;
        let mut host = Refusing(fake);
        let down = bring_up(&mut host, 501, LABEL, PLIST, Kick::Start, false).await.unwrap_err();
        assert!(matches!(down, Down::Bootstrap { tries: 1, ref ran } if ran.code == 122), "{down:?}");
    }

    /// launchd that answers every bootstrap with an error that won't pass.
    struct Refusing(Fake);

    impl Host for Refusing {
        fn launchctl(&mut self, args: &[&str]) -> Ran {
            if args.first() == Some(&"bootstrap") {
                return Fake::ran(122, "Bootstrap failed: 122: Path had bad ownership/permissions");
            }
            self.0.launchctl(args)
        }
        async fn pause(&mut self, wait: Duration) {
            self.0.pause(wait).await
        }
        async fn answering(&mut self) -> Option<u32> {
            self.0.answering().await
        }
    }

    #[tokio::test]
    async fn a_build_launchd_refuses_is_loaded_afresh_before_anything_else() {
        // As on 2026-11-01: an ad-hoc re-signed build, last exit 78, and no daemon for hours.
        let mut fake = Fake::loaded_and_running();
        (fake.state, fake.pid, fake.last_exit) = ("spawn scheduled", None, Some(EX_CONFIG));
        fake.refused_until_loads = 0;
        let up = up(&mut fake, Kick::Restart, false).await.unwrap();
        assert_eq!(up.reloaded, Some(Why::Refused));
        assert_eq!(fake.did("kickstart"), 0, "kicking a refused build only gets 78 again");
        assert_eq!(fake.did("bootstrap gui/501"), 1);
    }

    #[tokio::test]
    async fn a_build_refused_on_the_restart_itself_is_loaded_afresh() {
        let mut fake = Fake::loaded_and_running();
        // Replaced since the job was loaded: the kick gets 78; a fresh load runs it.
        fake.refused_until_loads = 1;
        fake.lwcr = true;
        let up = up(&mut fake, Kick::Restart, false).await.unwrap();
        assert_eq!(up.reloaded, Some(Why::Refused));
        let kick = fake.calls.iter().position(|c| c.starts_with("kickstart -k")).unwrap();
        let load = fake.calls.iter().position(|c| c.starts_with("bootstrap")).unwrap();
        assert!(kick < load, "{:?}", fake.calls);
    }

    #[tokio::test]
    async fn a_build_refused_even_loaded_afresh_is_given_up_on_and_said_so() {
        let mut fake = Fake::loaded_and_running();
        fake.refused_until_loads = 100;
        let down = up(&mut fake, Kick::Restart, false).await.unwrap_err();
        assert!(matches!(&down, Down::Refused { job } if job.last_exit == Some(EX_CONFIG)), "{down:?}");
        assert_eq!(fake.did("bootstrap"), RELOADS as usize);
    }

    #[tokio::test]
    async fn a_service_launchd_doesnt_hold_is_loaded() {
        let mut fake = Fake::loaded_and_running();
        (fake.loaded, fake.state, fake.pid) = (None, "", None);
        let up = up(&mut fake, Kick::Start, false).await.unwrap();
        assert_eq!(up.reloaded, Some(Why::NotLoaded));
        assert_eq!(fake.did("bootout"), 0, "nothing to take out");
    }

    #[tokio::test]
    async fn one_in_the_user_domain_moves_to_the_screens_session() {
        let mut fake = Fake::loaded_and_running();
        fake.loaded = Some("user/501".into());
        let up = up(&mut fake, Kick::Restart, false).await.unwrap();
        assert_eq!((up.reloaded, up.domain.as_str()), (Some(Why::Elsewhere), "gui/501"));
        assert!(fake.calls.contains(&"bootout user/501/im.equipe.daemon".to_owned()));
        assert!(fake.calls.contains(&format!("bootstrap gui/501 {PLIST}")));
    }

    #[tokio::test]
    async fn with_no_one_at_the_screen_the_user_domain_is_all_there_is() {
        let mut fake = Fake::loaded_and_running();
        (fake.gui, fake.loaded) = (false, Some("user/501".into()));
        let up = up(&mut fake, Kick::Restart, false).await.unwrap();
        assert_eq!((up.reloaded, up.domain.as_str()), (None, "user/501"));
        assert!(fake.calls.contains(&"kickstart -k user/501/im.equipe.daemon".to_owned()));
    }

    #[tokio::test]
    async fn a_start_another_won_is_taken_not_fought() {
        // Another start bootstraps first; this one's bootstrap then fails with 5.
        let mut fake = Fake::loaded_and_running();
        (fake.loaded, fake.state, fake.pid) = (None, "", None);
        let mut host = Raced { fake, raced: false };
        let up = bring_up(&mut host, 501, LABEL, PLIST, Kick::Start, false).await.unwrap();
        assert_eq!(up.reloaded, Some(Why::NotLoaded));
        assert_eq!(host.fake.did("bootout"), 0, "the other's job is left to run");
    }

    struct Raced {
        fake: Fake,
        raced: bool,
    }

    impl Host for Raced {
        fn launchctl(&mut self, args: &[&str]) -> Ran {
            if args.first() == Some(&"bootstrap") && !self.raced {
                self.raced = true;
                self.fake.launchctl(args);
                return Fake::ran(5, "Bootstrap failed: 5: Input/output error");
            }
            self.fake.launchctl(args)
        }
        async fn pause(&mut self, wait: Duration) {
            self.fake.pause(wait).await
        }
        async fn answering(&mut self) -> Option<u32> {
            self.fake.answering().await
        }
    }

    #[test]
    fn launchctls_own_words_are_read() {
        // As `launchctl print gui/501/<label>` said on this Mac, cut down.
        let running = "gui/501/im.equipe.test-11194 = {\n\tactive count = 1\n\tpath = /tmp/x.plist\n\ttype = LaunchAgent\n\tstate = running\n\n\tprogram = /bin/sleep\n\targuments = {\n\t\t/bin/sleep\n\t\t600\n\t}\n\n\tenvironment = {\n\t\tXPC_SERVICE_NAME => im.equipe.test-11194\n\t}\n\n\tdomain = gui/501 [100003]\n\truns = 1\n\tpid = 85052\n\tlast exit code = (never exited)\n\n\tresource coalition = {\n\t\tID = 16534\n\t\tstate = active\n\t}\n}\n";
        let job = Job::parse(running);
        assert_eq!(job, Job { state: "running".into(), pid: Some(85052), last_exit: None, lwcr: false });
        assert!(job.running() && !job.refuses());
        let refused = Job::parse("x = {\n\tstate = spawn scheduled\n\truns = 1\n\tlast exit code = 78: EX_CONFIG\n\tspawn type = daemon (3)\n}\n");
        assert_eq!((refused.state.as_str(), refused.pid, refused.last_exit), ("spawn scheduled", None, Some(78)));
        assert!(refused.refuses());
        assert_eq!(refused.abnormal_exit(), Some(78));
        let going = Job::parse("x = {\n\tstate = SIGTERMed\n\tpid = 88978\n}\n");
        assert!(going.going() && !going.running());
        let lwcr = Job::parse("x = {\n\tstate = not running\n\tproperties = runatload | needs LWCR update\n\tlast exit code = 0\n}\n");
        assert!(lwcr.refuses());
        assert_eq!(lwcr.abnormal_exit(), None);
    }

    /// launchd itself, with a throwaway job of its own — never the person's
    /// daemon: its daemon "answers" when its process is there.
    #[cfg(unix)]
    struct Throwaway {
        real: Launchd,
        target: String,
    }

    #[cfg(unix)]
    impl Host for Throwaway {
        fn launchctl(&mut self, args: &[&str]) -> Ran {
            assert!(!args.iter().any(|a| a.ends_with(LABEL)), "never the person's service: {args:?}");
            self.real.launchctl(args)
        }
        async fn pause(&mut self, wait: Duration) {
            self.real.pause(wait).await
        }
        async fn answering(&mut self) -> Option<u32> {
            let target = self.target.clone();
            let pid = print(self, &target)?.pid?;
            // SAFETY: kill with signal 0 only asks.
            (unsafe { libc::kill(pid as libc::pid_t, 0) } == 0).then_some(pid)
        }
    }

    /// Takes the throwaway job out of launchd, and its folder away, however the test ends.
    #[cfg(unix)]
    struct Cleanup {
        target: String,
        dir: std::path::PathBuf,
    }

    #[cfg(unix)]
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::process::Command::new("launchctl").args(["bootout", &self.target]).output();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[cfg(unix)]
    fn throwaway_plist(label: &str, script: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{label}</string>
  <key>ProgramArguments</key>
  <array><string>/bin/sh</string><string>-c</string><string>{script}</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>ExitTimeOut</key><integer>4</integer>
</dict>
</plist>
"#
        )
    }

    /// The whole flow against the real launchd, on a job named
    /// `im.equipe.test-<pid>` that only sleeps (and is slow to stop, as a
    /// daemon is), then one that exits 78. `cargo test -- --ignored real_launchd`.
    #[cfg(unix)]
    #[tokio::test]
    #[ignore = "loads and removes a throwaway launchd job in this user's session"]
    async fn real_launchd_restarts_in_place_reloads_through_error_5_and_gives_up_on_exit_78() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let uid = uid();
        let label = format!("im.equipe.test-{}", std::process::id());
        let dir = std::env::temp_dir().join(&label);
        std::fs::create_dir_all(&dir).unwrap();
        let plist = dir.join(format!("{label}.plist"));
        let (gui, user) = domains(uid);
        let domain = if std::process::Command::new("launchctl").args(["print", &gui]).output().unwrap().status.success() { gui } else { user };
        let target = format!("{domain}/{label}");
        let _cleanup = Cleanup { target: target.clone(), dir: dir.clone() };
        let mut host = Throwaway { real: Launchd { socket: dir.join("none.sock") }, target: target.clone() };
        let path = plist.to_string_lossy().into_owned();

        // Slow to stop: TERM is ignored until launchd's exit timeout.
        std::fs::write(&plist, throwaway_plist(&label, "trap '' TERM; while true; do sleep 1; done")).unwrap();
        let first = bring_up(&mut host, uid, &label, &path, Kick::Start, false).await.unwrap();
        assert_eq!(first.reloaded, Some(Why::NotLoaded));

        let again = bring_up(&mut host, uid, &label, &path, Kick::Restart, false).await.unwrap();
        assert_eq!(again.reloaded, None, "in place");
        assert_ne!(again.pid, first.pid, "a new process");

        // bootout returns while the old job is still `SIGTERMed`: bootstrap says 5 until it is gone.
        let changed = bring_up(&mut host, uid, &label, &path, Kick::Restart, true).await.unwrap();
        assert_eq!(changed.reloaded, Some(Why::Changed));
        assert_ne!(changed.pid, again.pid);

        std::fs::write(&plist, throwaway_plist(&label, "exit 78")).unwrap();
        let down = bring_up(&mut host, uid, &label, &path, Kick::Restart, true).await.unwrap_err();
        assert!(matches!(&down, Down::Refused { job } if job.last_exit == Some(EX_CONFIG)), "{down:?}");
    }
}
