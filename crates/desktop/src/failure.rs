//! Why the window isn't attached, in the terms the person can act on. The link
//! reports a [`Failure`]; the window turns it into a [`Guide`]: what happened,
//! what to do, and the lines to paste — one guide per cause, because "connection
//! failed" is no help to someone who has never used SSH.

use equipe_core::t;
use crate::install::INSTALL_COMMAND;
use crate::target::Endpoint;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The squad list is empty: nothing to follow yet.
    NoSquads,
    /// This Mac has no daemon listening.
    LocalDaemonOff { socket: String },
    /// The daemon (or the SSH connection) went away after we had it.
    Closed,
    /// The daemon accepted and never answered.
    NoAnswer,
    /// `ssh` was refused at the door: nothing listens on the port.
    RemoteLoginOff,
    /// No route, no name, no answer in time.
    Unreachable(String),
    /// The machine answered and didn't accept who we are.
    AuthFailed(String),
    /// The person closed the password / code box.
    SignInCancelled,
    /// The person declined the host's fingerprint (or it wasn't confirmed).
    HostKeyRefused,
    /// `StrictHostKeyChecking yes` and the host isn't in `known_hosts`.
    HostKeyStrict,
    /// The host's key is not the one remembered: not to be waved through.
    HostKeyChanged,
    /// `equipe` isn't on that machine.
    NoEquipe,
    /// The bridge couldn't reach or start the daemon; why, from its stderr.
    DaemonUnavailable(String),
    /// That machine's `equipe` is older than the bridge (`equipe attach`).
    NoBridge,
    /// The daemon speaks an older wire than this app.
    DaemonTooOld { daemon: u32 },
    /// The daemon speaks a newer wire than this app.
    AppTooOld { daemon: u32 },
    Other(String),
}

impl Failure {
    /// Failures that more tries won't fix by themselves: the window waits for
    /// the person (a "try again" button) instead of asking again every few seconds.
    pub fn waits_for_person(&self) -> bool {
        matches!(
            self,
            Self::SignInCancelled
                | Self::AuthFailed(_)
                | Self::HostKeyRefused
                | Self::HostKeyStrict
                | Self::HostKeyChanged
                | Self::AppTooOld { .. }
        )
    }

    /// The last line of `ssh`'s complaint, to show under the guide.
    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::Unreachable(d) | Self::AuthFailed(d) | Self::DaemonUnavailable(d) | Self::Other(d) if !d.is_empty() => Some(d),
            _ => None,
        }
    }
}

/// What to show for a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guide {
    pub title: String,
    pub text: String,
    /// What to do, in order.
    pub steps: Vec<String>,
    /// Lines to paste in a terminal (each with a copy button).
    pub commands: Vec<String>,
    /// The raw message from `ssh`, small, at the bottom.
    pub detail: Option<String>,
    /// A line of the banner (the guide itself is for when nothing was attached yet).
    pub short: String,
}

fn guide(title: impl Into<String>, short: impl Into<String>, text: impl Into<String>, steps: &[&str], commands: Vec<String>, failure: &Failure) -> Guide {
    Guide {
        title: title.into(),
        short: short.into(),
        text: text.into(),
        steps: steps.iter().map(|s| (*s).to_owned()).collect(),
        commands,
        detail: failure.detail().map(str::to_owned),
    }
}

impl Failure {
    pub fn guide(&self, endpoint: &Endpoint) -> Guide {
        let remote = match endpoint {
            Endpoint::Ssh(host) => Some(host),
            Endpoint::Local { .. } => None,
        };
        let name = endpoint.label();
        // `ssh` as the person would type it in a terminal for this host.
        let ssh_line = remote.map_or_else(String::new, |h| format!("ssh {}", h.args().iter().filter(|a| *a != "--").cloned().collect::<Vec<_>>().join(" ")));
        match self {
            Self::NoSquads if remote.is_some() => guide(
                t!("failure.no_squads_title"),
                t!("failure.no_squads_remote_short", name = name),
                t!("failure.no_squads_remote_text", name = name),
                &[t!("failure.no_squads_remote_step_ssh"), t!("failure.no_squads_remote_step_run"), t!("failure.no_squads_remote_step_wait")],
                vec![ssh_line, "equipe".into()],
                self,
            ),
            Self::NoSquads => guide(
                t!("failure.no_squads_title"),
                t!("failure.no_squads_short"),
                t!("failure.no_squads_text"),
                &[],
                vec!["equipe".into()],
                self,
            ),
            Self::LocalDaemonOff { socket } => guide(
                t!("failure.daemon_off_title"),
                t!("failure.daemon_off_short"),
                t!("failure.daemon_off_text"),
                &[],
                vec!["equipe".into()],
                &Self::Other(t!("failure.daemon_off_detail", socket = socket)),
            ),
            Self::Closed | Self::NoAnswer => guide(
                t!("failure.closed", name = name),
                t!("failure.closed", name = name),
                t!("failure.closed_text"),
                &[],
                Vec::new(),
                self,
            ),
            Self::RemoteLoginOff => guide(
                t!("failure.login_off_title", name = name),
                t!("failure.login_off_short", name = name),
                t!("failure.login_off_text", name = name),
                &[
                    t!("failure.login_off_step_mac"),
                    t!("failure.login_off_step_linux"),
                    t!("failure.login_off_step_port"),
                ],
                vec!["sudo apt install openssh-server && sudo systemctl enable --now ssh".into()],
                self,
            ),
            Self::Unreachable(_) => guide(
                t!("failure.unreachable", name = name),
                t!("failure.unreachable", name = name),
                t!("failure.unreachable_text", name = name),
                &[
                    t!("failure.unreachable_step_name"),
                    t!("failure.unreachable_step_awake"),
                    t!("failure.unreachable_step_vpn"),
                    t!("failure.unreachable_step_ssh"),
                ],
                if ssh_line.is_empty() { Vec::new() } else { vec![ssh_line] },
                self,
            ),
            Self::AuthFailed(_) => guide(
                t!("failure.auth_title", name = name),
                t!("failure.auth_short", name = name),
                t!("failure.auth_text"),
                &[
                    t!("failure.auth_step_user"),
                    t!("failure.auth_step_keygen"),
                    t!("failure.auth_step_copy_id"),
                    t!("failure.step_retry_below"),
                ],
                match remote {
                    Some(h) => vec!["ssh-keygen -t ed25519".into(), format!("ssh-copy-id {}", h.args().iter().filter(|a| *a != "--").cloned().collect::<Vec<_>>().join(" "))],
                    None => Vec::new(),
                },
                self,
            ),
            Self::SignInCancelled => guide(
                t!("failure.sign_in_cancelled"),
                t!("failure.sign_in_cancelled"),
                t!("failure.sign_in_cancelled_text", name = name),
                &[t!("failure.step_retry_when_ready")],
                Vec::new(),
                self,
            ),
            Self::HostKeyRefused => guide(
                t!("failure.host_key_refused"),
                t!("failure.host_key_refused"),
                t!("failure.host_key_refused_text", name = name),
                &[t!("failure.host_key_refused_step_retry"), t!("failure.host_key_refused_step_compare")],
                Vec::new(),
                self,
            ),
            Self::HostKeyStrict => guide(
                t!("failure.host_key_strict_title"),
                t!("failure.host_key_strict_short"),
                t!("failure.host_key_strict_text", name = name),
                &[t!("failure.host_key_strict_step_ssh"), t!("failure.step_retry")],
                if ssh_line.is_empty() { Vec::new() } else { vec![ssh_line] },
                self,
            ),
            Self::HostKeyChanged => guide(
                t!("failure.host_key_changed_title", name = name),
                t!("failure.host_key_changed_short", name = name),
                t!("failure.host_key_changed_text"),
                &[
                    t!("failure.host_key_changed_step_ask"),
                    t!("failure.host_key_changed_step_remove"),
                ],
                remote.map(|h| vec![format!("ssh-keygen -R {}", h.host)]).unwrap_or_default(),
                self,
            ),
            Self::NoEquipe => guide(
                t!("failure.no_equipe", name = name),
                t!("failure.no_equipe", name = name),
                t!("failure.no_equipe_text", name = name),
                &[
                    t!("failure.no_equipe_step_ssh"),
                    t!("failure.no_equipe_step_install"),
                    t!("failure.no_equipe_step_wait"),
                ],
                if ssh_line.is_empty() { vec![INSTALL_COMMAND.into()] } else { vec![ssh_line, INSTALL_COMMAND.into()] },
                self,
            ),
            Self::DaemonUnavailable(_) => guide(
                t!("failure.daemon_unavailable", name = name),
                t!("failure.daemon_unavailable", name = name),
                t!("failure.daemon_unavailable_text", name = name),
                &[t!("failure.daemon_unavailable_step")],
                if ssh_line.is_empty() { vec!["equipe daemon start".into()] } else { vec![ssh_line, "equipe daemon start".into()] },
                self,
            ),
            Self::NoBridge => guide(
                t!("failure.no_bridge_title", name = name),
                t!("failure.update_remote_short", name = name),
                t!("failure.no_bridge_text", name = name),
                &[t!("failure.update_remote_step")],
                if ssh_line.is_empty() { vec!["equipe update".into(), "equipe daemon restart".into()] } else { vec![ssh_line, "equipe update".into(), "equipe daemon restart".into()] },
                self,
            ),
            Self::DaemonTooOld { daemon } => guide(
                t!("failure.daemon_too_old_title", name = name),
                t!("failure.update_remote_short", name = name),
                t!("failure.daemon_too_old_text", name = name, daemon = daemon, app = equipe_core::protocol::rpc::PROTOCOL),
                &[t!("failure.update_remote_step")],
                if ssh_line.is_empty() { vec!["equipe update".into(), "equipe daemon restart".into()] } else { vec![ssh_line, "equipe update".into(), "equipe daemon restart".into()] },
                self,
            ),
            Self::AppTooOld { daemon } => guide(
                t!("failure.app_too_old_title"),
                t!("failure.app_too_old_short"),
                t!("failure.app_too_old_text", name = name, daemon = daemon, app = equipe_core::protocol::rpc::PROTOCOL),
                &[t!("failure.app_too_old_step_menu"), t!("failure.app_too_old_step_site")],
                Vec::new(),
                self,
            ),
            Self::Other(_) => guide(
                t!("failure.other_error", name = name),
                t!("failure.other_error", name = name),
                t!("failure.other_text"),
                &[],
                if ssh_line.is_empty() { Vec::new() } else { vec![ssh_line] },
                self,
            ),
        }
    }
}

/// What `ssh` (or the `equipe attach --stdio` it ran) left behind, as a cause.
/// `status` is the exit code (`None`: killed by a signal); `stderr` the lines
/// it wrote, oldest first.
pub fn classify_exit(status: Option<i32>, stderr: &[String]) -> Failure {
    let text = stderr.join("\n").to_lowercase();
    let last = stderr.iter().rev().map(|l| l.trim()).find(|l| !l.is_empty()).unwrap_or("").to_owned();
    let has = |needle: &str| text.contains(needle);
    if has("identification has changed") {
        return Failure::HostKeyChanged;
    }
    if has("host key verification failed") {
        return if has("strict checking") { Failure::HostKeyStrict } else { Failure::HostKeyRefused };
    }
    match status {
        Some(0) => return Failure::Closed,
        Some(69) => return Failure::DaemonUnavailable(last.trim_start_matches("equipe attach: ").to_owned()),
        Some(127) => return Failure::NoEquipe,
        // clap's complaint about a subcommand an older equipe doesn't have.
        _ if has("unrecognized subcommand") || has("unexpected argument 'attach'") => return Failure::NoBridge,
        _ => {}
    }
    if has("permission denied") || has("too many authentication failures") || has("no more authentication methods") {
        return Failure::AuthFailed(last);
    }
    if has("connection refused") {
        return Failure::RemoteLoginOff;
    }
    const UNREACHABLE: [&str; 9] = [
        "could not resolve hostname",
        "name or service not known",
        "nodename nor servname",
        "timed out",
        "no route to host",
        "network is unreachable",
        "connection closed by",
        "connection reset",
        "broken pipe",
    ];
    if UNREACHABLE.iter().any(|n| has(n)) {
        return Failure::Unreachable(last);
    }
    if has("command not found") && has("equipe") {
        return Failure::NoEquipe;
    }
    Failure::Other(if last.is_empty() { t!("failure.ssh_exited", status = status.map_or(t!("failure.signal").to_owned(), |s| s.to_string())) } else { last })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::SshHost;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_owned).collect()
    }

    #[test]
    fn each_ssh_ending_gets_its_own_cause() {
        let c = |status, text: &str| classify_exit(status, &lines(text));
        assert_eq!(c(Some(255), "ssh: connect to host mini port 22: Connection refused"), Failure::RemoteLoginOff);
        assert!(matches!(c(Some(255), "ssh: connect to host mini port 22: Operation timed out"), Failure::Unreachable(_)));
        assert!(matches!(c(Some(255), "ssh: Could not resolve hostname mini: nodename nor servname provided"), Failure::Unreachable(_)));
        assert!(matches!(c(Some(255), "me@mini: Permission denied (publickey,password)."), Failure::AuthFailed(_)));
        assert_eq!(c(Some(255), "Host key verification failed."), Failure::HostKeyRefused);
        assert_eq!(c(Some(255), "No ED25519 host key is known for x and you have requested strict checking.\nHost key verification failed."), Failure::HostKeyStrict);
        assert_eq!(
            c(Some(255), "@@@ WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED! @@@\nHost key verification failed."),
            Failure::HostKeyChanged
        );
        assert_eq!(c(Some(127), "sh: 1: exec: equipe: not found"), Failure::NoEquipe);
        assert_eq!(c(Some(69), "equipe attach: the daemon isn't running"), Failure::DaemonUnavailable("the daemon isn't running".into()));
        assert_eq!(c(Some(2), "error: unrecognized subcommand 'attach'\n\nUsage: equipe [OPTIONS] [COMMAND]"), Failure::NoBridge);
        assert_eq!(c(Some(0), ""), Failure::Closed);
        assert!(matches!(c(Some(1), "something odd"), Failure::Other(m) if m == "something odd"));
        assert!(matches!(c(None, ""), Failure::Other(_)));
    }

    #[test]
    fn only_what_the_person_must_fix_stops_the_retries() {
        assert!(Failure::AuthFailed(String::new()).waits_for_person());
        assert!(Failure::HostKeyChanged.waits_for_person());
        assert!(Failure::SignInCancelled.waits_for_person());
        assert!(!Failure::RemoteLoginOff.waits_for_person());
        assert!(!Failure::NoEquipe.waits_for_person());
        assert!(!Failure::Closed.waits_for_person());
    }

    #[test]
    fn every_cause_has_a_distinct_guide_with_the_host_in_its_commands() {
        let host = Endpoint::Ssh(SshHost::parse("me@mini:2222").unwrap());
        let all = [
            Failure::NoSquads,
            Failure::Closed,
            Failure::RemoteLoginOff,
            Failure::Unreachable("x".into()),
            Failure::AuthFailed("x".into()),
            Failure::SignInCancelled,
            Failure::HostKeyRefused,
            Failure::HostKeyStrict,
            Failure::HostKeyChanged,
            Failure::NoEquipe,
            Failure::DaemonUnavailable("x".into()),
            Failure::NoBridge,
            Failure::DaemonTooOld { daemon: 0 },
            Failure::AppTooOld { daemon: 9 },
            Failure::Other("x".into()),
        ];
        let titles: std::collections::HashSet<_> = all.iter().map(|f| f.guide(&host).title).collect();
        assert_eq!(titles.len(), all.len(), "no two causes share a title");

        let copy = Failure::AuthFailed(String::new()).guide(&host).commands;
        assert_eq!(copy, ["ssh-keygen -t ed25519", "ssh-copy-id -p 2222 me@mini"]);
        let install = Failure::NoEquipe.guide(&host).commands;
        assert_eq!(install[0], "ssh -p 2222 me@mini");
        assert!(install[1].contains("install.sh"));
        assert!(Failure::RemoteLoginOff.guide(&host).steps.iter().any(|s| s.contains("원격 로그인")));
    }
}
