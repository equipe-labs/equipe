//! Why the window isn't attached, in the terms the person can act on. The link
//! reports a [`Failure`]; the window turns it into a [`Guide`]: what happened,
//! what to do, and the lines to paste — one guide per cause, because "connection
//! failed" is no help to someone who has never used SSH.

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
                "아직 스쿼드가 없습니다",
                format!("{name}에 스쿼드가 없습니다"),
                format!("{name}의 데몬에는 스쿼드가 하나도 없습니다. 그 기계의 프로젝트 폴더에서 equipe를 한 번 실행해 스쿼드를 만드세요."),
                &["터미널에서 아래 첫 줄로 그 기계에 들어갑니다", "프로젝트 폴더로 이동해 `equipe`를 실행합니다", "스쿼드가 생기면 이 창이 알아서 연결합니다"],
                vec![ssh_line, "equipe".into()],
                self,
            ),
            Self::NoSquads => guide(
                "아직 스쿼드가 없습니다",
                "스쿼드가 없습니다",
                "프로젝트 폴더에서 `equipe`를 한 번 실행하면 스쿼드가 만들어집니다. 그러면 이 창이 알아서 연결합니다.",
                &[],
                vec!["equipe".into()],
                self,
            ),
            Self::LocalDaemonOff { socket } => guide(
                "equipe 데몬이 실행 중이 아닙니다",
                "데몬이 꺼져 있습니다",
                "터미널에서 `equipe`를 실행하면 데몬이 시작됩니다. 켜지는 즉시 이 창이 알아서 연결합니다.",
                &[],
                vec!["equipe".into()],
                &Self::Other(format!("{socket}을 찾아봤습니다")),
            ),
            Self::Closed | Self::NoAnswer => guide(
                format!("{name}과의 연결이 끊겼습니다"),
                format!("{name}과의 연결이 끊겼습니다"),
                "다시 연결하는 중입니다. 데몬이 켜지거나 네트워크가 돌아오면 자동으로 이어집니다.",
                &[],
                Vec::new(),
                self,
            ),
            Self::RemoteLoginOff => guide(
                format!("{name}에 접속할 수 없습니다"),
                format!("{name}에서 SSH 접속을 받지 않습니다"),
                format!("{name}이(가) SSH 접속을 받고 있지 않습니다 — 그 기계의 '원격 로그인'(SSH 서버)이 꺼져 있을 가능성이 큽니다. 기계 앞에서 한 번만 켜 두면 됩니다."),
                &[
                    "맥: 시스템 설정 → 일반 → 공유 → '원격 로그인'을 켭니다 (macOS 12 이하: 시스템 환경설정 → 공유). 접속을 허용할 사용자도 확인하세요",
                    "리눅스: SSH 서버를 설치하고 켭니다 — 아래 명령은 Debian/Ubuntu 기준 (다른 배포판은 sshd를 켜는 방법을 찾아보세요)",
                    "포트를 바꿨다면 `host:포트` 형식으로 다시 입력하세요",
                ],
                vec!["sudo apt install openssh-server && sudo systemctl enable --now ssh".into()],
                self,
            ),
            Self::Unreachable(_) => guide(
                format!("{name}에 닿지 않습니다"),
                format!("{name}에 닿지 않습니다"),
                format!("{name}에 연결하지 못했습니다. 이름이나 주소가 틀렸거나, 그 기계가 꺼져(잠자기) 있거나, 다른 네트워크에 있을 수 있습니다."),
                &[
                    "이름·주소를 확인하세요 (오타, `.local` 이름은 같은 Wi-Fi에서만 보입니다)",
                    "그 기계가 켜져 있고 잠자기 상태가 아닌지 확인하세요",
                    "다른 네트워크라면 VPN이나 Tailscale 같은 사설망에 연결돼 있어야 합니다",
                    "터미널에서 같은 호스트로 `ssh`가 되는지 확인해 보세요",
                ],
                if ssh_line.is_empty() { Vec::new() } else { vec![ssh_line] },
                self,
            ),
            Self::AuthFailed(_) => guide(
                format!("{name}이(가) 로그인을 받아 주지 않습니다"),
                format!("{name} 인증에 실패했습니다"),
                "사용자 이름이나 비밀번호가 맞지 않거나, 이 맥의 SSH 키가 그 기계에 등록돼 있지 않습니다. 키로 로그인하면 비밀번호를 매번 묻지 않아 가장 편합니다.",
                &[
                    "사용자 이름이 맞는지 확인하세요 (`user@host` 형식으로 입력할 수 있습니다)",
                    "키가 아직 없다면 만듭니다 (이미 `~/.ssh/id_ed25519`가 있으면 건너뜁니다)",
                    "키를 그 기계에 등록합니다 — 이때만 그 기계의 비밀번호를 묻습니다",
                    "끝나면 아래 '다시 시도'를 누르세요",
                ],
                match remote {
                    Some(h) => vec!["ssh-keygen -t ed25519".into(), format!("ssh-copy-id {}", h.args().iter().filter(|a| *a != "--").cloned().collect::<Vec<_>>().join(" "))],
                    None => Vec::new(),
                },
                self,
            ),
            Self::SignInCancelled => guide(
                "로그인을 취소했습니다",
                "로그인을 취소했습니다",
                format!("{name}에 들어가려면 비밀번호나 인증 코드가 필요합니다. 비밀번호는 저장하지 않으며, 연결을 이어 가는 동안만 SSH가 기억합니다."),
                &["준비되면 '다시 시도'를 누르세요"],
                Vec::new(),
                self,
            ),
            Self::HostKeyRefused => guide(
                "호스트 키를 확인하지 않았습니다",
                "호스트 키를 확인하지 않았습니다",
                format!("{name}은(는) 이 맥이 처음 보는 기계입니다. 지문(fingerprint)이 그 기계의 것이 맞는지 확인한 뒤에만 연결합니다."),
                &["'다시 시도'를 누르면 지문을 다시 보여 드립니다", "그 기계에서 `ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub`를 실행한 결과와 같은지 비교하세요"],
                Vec::new(),
                self,
            ),
            Self::HostKeyStrict => guide(
                "처음 보는 호스트 키가 허용되지 않습니다",
                "알 수 없는 호스트 키",
                format!("~/.ssh/config가 `StrictHostKeyChecking yes`라서 {name}의 키를 자동으로 등록하지 않습니다."),
                &["터미널에서 아래 명령으로 한 번 접속해 지문을 확인하고 등록하세요", "끝나면 '다시 시도'를 누르세요"],
                if ssh_line.is_empty() { Vec::new() } else { vec![ssh_line] },
                self,
            ),
            Self::HostKeyChanged => guide(
                format!("{name}의 호스트 키가 달라졌습니다"),
                format!("{name}의 호스트 키가 바뀌었습니다"),
                "이 맥이 기억하는 키와 지금 그 주소가 내놓는 키가 다릅니다. 기계를 새로 설치했거나 주소가 다른 기계를 가리키는 것일 수 있지만, 누군가 중간에 끼어든 것일 수도 있어서 앱은 이 연결을 허용하지 않습니다.",
                &[
                    "기계 관리자에게 키가 바뀐 이유를 확인하세요",
                    "정말 바뀐 것이 맞다면 옛 키를 지우고 다시 시도합니다",
                ],
                remote.map(|h| vec![format!("ssh-keygen -R {}", h.host)]).unwrap_or_default(),
                self,
            ),
            Self::NoEquipe => guide(
                format!("{name}에 equipe가 없습니다"),
                format!("{name}에 equipe가 없습니다"),
                format!("{name}에서 `equipe` 명령을 찾지 못했습니다. 이 앱은 그 기계의 equipe 데몬을 보여 주는 창이라, 그 기계에도 equipe가 설치돼 있어야 합니다."),
                &[
                    "터미널에서 그 기계에 들어갑니다",
                    "아래 한 줄로 equipe를 설치합니다 (Homebrew를 쓰면 `brew install equipe-labs/tap/equipe`)",
                    "설치가 끝나면 이 창이 알아서 다시 연결합니다 (`~/.local/bin`, `~/.cargo/bin`, Homebrew 폴더는 자동으로 찾습니다)",
                ],
                if ssh_line.is_empty() { vec![INSTALL_COMMAND.into()] } else { vec![ssh_line, INSTALL_COMMAND.into()] },
                self,
            ),
            Self::DaemonUnavailable(_) => guide(
                format!("{name}의 데몬을 시작하지 못했습니다"),
                format!("{name}의 데몬을 시작하지 못했습니다"),
                format!("{name}에는 equipe가 있지만 데몬에 닿지 못했고 시작도 되지 않았습니다. 아래 메시지가 이유입니다."),
                &["그 기계에서 `equipe`를 직접 실행해 보고, 같은 메시지가 나오는지 확인하세요"],
                if ssh_line.is_empty() { vec!["equipe daemon start".into()] } else { vec![ssh_line, "equipe daemon start".into()] },
                self,
            ),
            Self::NoBridge => guide(
                format!("{name}의 equipe가 너무 오래됐습니다"),
                format!("{name}의 equipe를 업데이트하세요"),
                format!("{name}의 equipe에는 이 앱이 쓰는 `equipe attach`가 없습니다. 그 기계의 equipe를 업데이트하면 이어집니다."),
                &["그 기계에서 업데이트하고 데몬을 다시 시작합니다 — 작업 중인 멤버는 잠시 멈췄다 이어집니다"],
                if ssh_line.is_empty() { vec!["equipe update".into(), "equipe daemon restart".into()] } else { vec![ssh_line, "equipe update".into(), "equipe daemon restart".into()] },
                self,
            ),
            Self::DaemonTooOld { daemon } => guide(
                format!("{name}의 equipe가 이 앱보다 오래됐습니다"),
                format!("{name}의 equipe를 업데이트하세요"),
                format!("{name}의 데몬은 프로토콜 {daemon}번이고 이 앱은 {}번입니다. 그 기계의 equipe를 업데이트하면 이어집니다.", equipe_core::protocol::rpc::PROTOCOL),
                &["그 기계에서 업데이트하고 데몬을 다시 시작합니다 — 작업 중인 멤버는 잠시 멈췄다 이어집니다"],
                if ssh_line.is_empty() { vec!["equipe update".into(), "equipe daemon restart".into()] } else { vec![ssh_line, "equipe update".into(), "equipe daemon restart".into()] },
                self,
            ),
            Self::AppTooOld { daemon } => guide(
                "이 앱을 업데이트하세요",
                "이 앱이 더 오래됐습니다",
                format!("{name}의 데몬은 프로토콜 {daemon}번이고 이 앱은 {}번이라 이 앱이 따라가지 못합니다.", equipe_core::protocol::rpc::PROTOCOL),
                &["메뉴 equipe → '업데이트 확인…'으로 앱을 업데이트하세요", "또는 equipe.im에서 최신 앱을 받으세요"],
                Vec::new(),
                self,
            ),
            Self::Other(_) => guide(
                format!("{name}에 연결하지 못했습니다"),
                format!("{name}에 연결하지 못했습니다"),
                "원인을 정확히 알 수 없습니다. 아래 메시지를 확인하세요. 터미널에서 같은 호스트로 직접 접속해 보면 더 자세히 나옵니다.",
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
    Failure::Other(if last.is_empty() { format!("ssh가 종료됐습니다 (상태 {})", status.map_or("신호".to_owned(), |s| s.to_string())) } else { last })
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
