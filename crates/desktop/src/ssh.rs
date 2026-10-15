//! Running `equipe attach --stdio` on another machine with the system `ssh`.
//!
//! The person's own SSH setup applies untouched — `~/.ssh/config` (ProxyJump,
//! port, IdentityFile, agents, 1Password's agent, …) is read by `ssh` itself.
//! The app only adds: no terminal (`-T`), keep-alives, one shared connection
//! (`ControlMaster`) so a second command costs no second sign-in, and
//! `SSH_ASKPASS` pointing back at itself for the questions `ssh` asks
//! (see `askpass.rs`). It never auto-accepts a host key and stores no password.

use equipe_core::t;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::askpass;
use crate::failure::{Failure, classify_exit};
use crate::target::SshHost;

/// Where a command over SSH should look for `equipe`: it gets a non-login
/// shell, whose PATH lacks the folders the installers use. `install.sh`
/// (`~/.local/bin`), `cargo install` (`~/.cargo/bin`), Homebrew (both prefixes).
const PATH_PREFIX: &str = "$HOME/.local/bin:$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin";

/// What the remote login shell is asked to run. The login shell may be bash,
/// zsh, fish, … so the line is written to mean the same in all of them: one
/// `sh -c '…'` whose script is POSIX and has nothing the outer shell would
/// expand (single quotes keep `$HOME` and `$PATH` for `sh` — fish's `$PATH`
/// is a list, and `VAR=x cmd` isn't understood everywhere).
pub fn attach_command() -> String {
    format!("sh -c 'PATH=\"{PATH_PREFIX}:$PATH\"; export PATH; exec equipe attach --stdio'")
}

/// A one-off command (its script goes in on standard input, so nothing is
/// quoted for the outer shell): `sh -s`.
const SCRIPT_COMMAND: &str = "sh -s";

/// `'…'` for a POSIX shell, `'` written `'\''`.
pub fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// What the helper needs to find the app again.
#[derive(Debug, Clone)]
pub struct Askpass {
    pub exe: PathBuf,
    pub socket: PathBuf,
    pub token: String,
}

/// How to run `ssh`.
#[derive(Debug, Clone)]
pub struct Launcher {
    pub program: PathBuf,
    /// Where the shared connections' sockets live: short and private.
    pub control_dir: PathBuf,
    pub askpass: Option<Askpass>,
    /// Extra `ssh` options from `$EQUIPE_SSH_OPTIONS` (e.g. `-F /path/config`), for
    /// the odd setup and for tests; placed before the host.
    pub extra: Vec<String>,
}

impl Launcher {
    pub fn system(askpass: Option<Askpass>) -> Self {
        let control_dir = askpass::private_dir().unwrap_or_else(|_| PathBuf::from("/tmp"));
        let extra = std::env::var("EQUIPE_SSH_OPTIONS").map(|o| o.split_whitespace().map(str::to_owned).collect()).unwrap_or_default();
        Self { program: PathBuf::from("ssh"), control_dir, askpass, extra }
    }

    /// The options every run gets, before the host.
    pub fn options(&self) -> Vec<String> {
        let control = self.control_dir.join("c-%C");
        let mut options: Vec<String> = [
            "-T",
            "-o",
            "ServerAliveInterval=15",
            "-o",
            "ServerAliveCountMax=3",
            "-o",
            "ConnectTimeout=15",
            "-o",
            "ControlMaster=auto",
            "-o",
            &format!("ControlPath={}", control.display()),
            "-o",
            "ControlPersist=600",
            // One question at a time; a wrong password ends in "Permission
            // denied" (a guide) rather than the same box again and again.
            "-o",
            "NumberOfPasswordPrompts=2",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        // With no box to ask in, never let ssh ask on a terminal that may be there.
        if self.askpass.is_none() {
            options.extend(["-o".to_owned(), "BatchMode=yes".to_owned()]);
        }
        options.extend(self.extra.iter().cloned());
        options
    }

    fn command(&self, host: &SshHost, remote: &str) -> Command {
        let mut command = Command::new(&self.program);
        command.args(self.options()).args(host.args()).arg(remote);
        if let Some(askpass) = &self.askpass {
            command
                .env("SSH_ASKPASS", &askpass.exe)
                .env("SSH_ASKPASS_REQUIRE", "force")
                .env(askpass::ENV_SOCKET, &askpass.socket)
                .env(askpass::ENV_TOKEN, &askpass.token);
        }
        command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        command
    }

    /// Start `equipe attach --stdio` on `host`.
    pub fn attach(&self, host: &SshHost) -> std::io::Result<(Process, ChildStdout, ChildStdin)> {
        let mut child = self.command(host, &attach_command()).spawn()?;
        let stdout = child.stdout.take().ok_or_else(|| std::io::Error::other("no stdout"))?;
        let stdin = child.stdin.take().ok_or_else(|| std::io::Error::other("no stdin"))?;
        Ok((Process::new(child), stdout, stdin))
    }

    /// Run `script` (POSIX `sh`) on `host`, over the shared connection; its
    /// standard output, or why not.
    pub async fn run(&self, host: &SshHost, script: &str) -> Result<Vec<u8>, Failure> {
        let mut child = self.command(host, SCRIPT_COMMAND).spawn().map_err(|e| Failure::Other(e.to_string()))?;
        let mut stdin = child.stdin.take().ok_or_else(|| Failure::Other("no stdin".into()))?;
        let mut stdout = child.stdout.take().ok_or_else(|| Failure::Other("no stdout".into()))?;
        let script = format!("PATH=\"{PATH_PREFIX}:$PATH\"; export PATH\n{script}\n");
        let mut process = Process::new(child);
        let feed = async move {
            let _ = stdin.write_all(script.as_bytes()).await;
            let _ = stdin.shutdown().await;
        };
        let mut out = Vec::new();
        let read = async { stdout.read_to_end(&mut out).await };
        let (_, read) = tokio::join!(feed, read);
        read.map_err(|e| Failure::Other(e.to_string()))?;
        match process.finish(false).await {
            Failure::Closed => Ok(out),
            failure => Err(failure),
        }
    }

    /// Stream the file at `path` into `equipe receive` on `host`, over the
    /// shared connection — a channel of its own, raw bytes, so a big file
    /// never becomes one giant line on the daemon's. `name` must already be
    /// tidy (letters, digits, `-`, `_`, one dot): it goes on the command line.
    /// `sent` hears how many bytes have gone. Where the far side kept it.
    pub async fn send_file(&self, host: &SshHost, path: &Path, name: &str, kind: &str, sent: impl Fn(u64)) -> Result<String, Failure> {
        use tokio::io::AsyncReadExt as _;
        let tidy = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.')) && !s.starts_with(['-', '.']);
        if !tidy(name) || !kind.chars().all(|c| c.is_ascii_lowercase()) {
            return Err(Failure::Other(format!("not a name to send: {name}")));
        }
        let mut file = tokio::fs::File::open(path).await.map_err(|e| Failure::Other(e.to_string()))?;
        let size = file.metadata().await.map_err(|e| Failure::Other(e.to_string()))?.len();
        let remote = format!("sh -c 'PATH=\"{PATH_PREFIX}:$PATH\"; export PATH; exec equipe receive --kind {kind} --size {size} --name {name}'");
        let mut child = self.command(host, &remote).spawn().map_err(|e| Failure::Other(e.to_string()))?;
        let mut stdin = child.stdin.take().ok_or_else(|| Failure::Other("no stdin".into()))?;
        let mut stdout = child.stdout.take().ok_or_else(|| Failure::Other("no stdout".into()))?;
        let mut process = Process::new(child);
        let feed = async move {
            let mut buffer = vec![0u8; 256 << 10];
            let mut gone = 0u64;
            loop {
                let n = file.read(&mut buffer).await.map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                stdin.write_all(&buffer[..n]).await.map_err(|e| e.to_string())?;
                gone += n as u64;
                sent(gone);
            }
            stdin.shutdown().await.map_err(|e| e.to_string())
        };
        let mut out = Vec::new();
        let read = async { stdout.read_to_end(&mut out).await };
        let (fed, _) = tokio::join!(feed, read);
        match process.finish(false).await {
            Failure::Closed if fed.is_ok() => {
                let kept = String::from_utf8_lossy(&out).trim().to_owned();
                if kept.starts_with('/') { Ok(kept) } else { Err(Failure::Other(t!("failure.no_kept_path").into())) }
            }
            Failure::Closed => Err(Failure::Other(fed.err().unwrap_or_default())),
            failure => Err(failure),
        }
    }

    /// Close the shared connection to `host` (a dead one after the network
    /// changed or the Mac slept would otherwise be reused until it times out).
    pub async fn drop_shared_connection(&self, host: &SshHost) {
        let mut command = Command::new(&self.program);
        command
            .args(["-O", "exit", "-o"])
            .arg(format!("ControlPath={}", self.control_dir.join("c-%C").display()))
            .args(host.args())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if let Ok(mut child) = command.spawn() {
            let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
        }
    }
}

/// A running `ssh`, with what it says on standard error kept for later.
pub struct Process {
    child: Child,
    stderr: Arc<Mutex<Vec<String>>>,
    reader: Option<tokio::task::JoinHandle<()>>,
}

/// More than this many lines of complaint are not kept.
const STDERR_KEEP: usize = 40;

impl Process {
    fn new(mut child: Child) -> Self {
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let reader = child.stderr.take().map(|pipe| {
            let kept = stderr.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(pipe).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut kept = kept.lock().unwrap();
                    if kept.len() >= STDERR_KEEP {
                        kept.remove(0);
                    }
                    kept.push(line);
                }
            })
        });
        Self { child, stderr, reader }
    }

    pub fn stderr(&self) -> Vec<String> {
        self.stderr.lock().unwrap().clone()
    }

    /// Wait for the end (it is ending — stdout closed — so not for long) and say why.
    /// `cancelled`: the person closed a question box, which reads as a refusal.
    pub async fn finish(&mut self, cancelled: bool) -> Failure {
        let status = match tokio::time::timeout(Duration::from_secs(3), self.child.wait()).await {
            Ok(Ok(status)) => status.code(),
            _ => {
                let _ = self.child.kill().await;
                None
            }
        };
        if let Some(reader) = self.reader.take() {
            let _ = tokio::time::timeout(Duration::from_millis(500), reader).await;
        }
        let failure = classify_exit(status, &self.stderr());
        match failure {
            Failure::AuthFailed(_) | Failure::Other(_) if cancelled => Failure::SignInCancelled,
            failure => failure,
        }
    }

    /// End it now.
    pub async fn kill(&mut self) {
        let _ = self.child.kill().await;
    }
}

/// The folder the app can keep short-lived files in.
pub fn scratch_dir() -> PathBuf {
    askpass::private_dir().unwrap_or_else(|_| std::env::temp_dir())
}

/// A file's name part safe to use locally: just the extension and a stem.
pub fn safe_name(path: &Path) -> String {
    let stem: String = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file").chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).take(40).collect();
    let ext: String = path.extension().and_then(|s| s.to_str()).unwrap_or("").chars().filter(char::is_ascii_alphanumeric).take(8).collect();
    match (stem.is_empty(), ext.is_empty()) {
        (true, true) => "file".into(),
        (true, false) => format!("file.{ext}"),
        (false, true) => stem,
        (false, false) => format!("{stem}.{ext}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as Std;

    fn host() -> SshHost {
        SshHost::parse("me@mini:2222").unwrap()
    }

    #[test]
    fn the_line_has_the_options_the_design_needs_and_the_host_last_but_one() {
        let launcher = Launcher { program: "ssh".into(), control_dir: "/tmp/x".into(), askpass: None, extra: vec![] };
        let command = launcher.command(&host(), "REMOTE");
        let args: Vec<_> = command.as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args[0], "-T");
        assert!(args.windows(2).any(|w| w == ["-o", "ControlMaster=auto"]));
        assert!(args.windows(2).any(|w| w == ["-o", "ControlPath=/tmp/x/c-%C"]));
        assert!(args.contains(&"BatchMode=yes".to_owned()), "with no box to ask in, ssh must not ask on a terminal");
        assert!(!args.iter().any(|a| a.contains("StrictHostKeyChecking")), "the person's host-key policy stays theirs");
        assert_eq!(args[args.len() - 5..args.len() - 1], ["-p", "2222", "--", "me@mini"]);
        assert_eq!(args.last().unwrap(), "REMOTE");
    }

    #[test]
    fn asks_come_back_to_the_app() {
        let askpass = Askpass { exe: "/Apps/equipe".into(), socket: "/tmp/s.sock".into(), token: "t0k".into() };
        let launcher = Launcher { program: "ssh".into(), control_dir: "/tmp/x".into(), askpass: Some(askpass), extra: vec![] };
        let command = launcher.command(&host(), "REMOTE");
        let env: std::collections::HashMap<_, _> = command.as_std().get_envs().map(|(k, v)| (k.to_string_lossy().into_owned(), v.map(|v| v.to_string_lossy().into_owned()))).collect();
        assert!(!command.as_std().get_args().any(|a| a.to_string_lossy().starts_with("BatchMode")), "ssh must be allowed to ask");
        assert_eq!(env["SSH_ASKPASS"].as_deref(), Some("/Apps/equipe"));
        assert_eq!(env["SSH_ASKPASS_REQUIRE"].as_deref(), Some("force"));
        assert_eq!(env[askpass::ENV_TOKEN].as_deref(), Some("t0k"));
    }

    #[test]
    fn quoting_survives_a_quote() {
        assert_eq!(quote("/a/b c"), "'/a/b c'");
        assert_eq!(quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn local_names_are_reduced_to_safe_characters() {
        assert_eq!(safe_name(Path::new("/x/../.. /ev il$.png")), "evil.png");
        assert_eq!(safe_name(Path::new("/x/preview-1.html")), "preview-1.html");
        assert_eq!(safe_name(Path::new("/")), "file");
    }

    /// A big file goes over a channel of its own into `equipe receive`: an
    /// `ssh` stand-in runs the remote line here, and a stand-in `equipe`
    /// keeps what arrives and says where.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_file_streams_into_equipe_receive_and_its_path_comes_back() {
        use std::os::unix::fs::PermissionsExt as _;
        let home = std::env::temp_dir().join(format!("eq-send-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".local/bin")).unwrap();
        let script = |path: &Path, body: &str| {
            std::fs::write(path, body).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        // `ssh [options] -- host <line>`: run the line with this HOME.
        let fake_ssh = home.join("ssh");
        script(&fake_ssh, &format!("#!/bin/sh\nfor last; do :; done\nHOME='{}' exec /bin/sh -c \"$last\"\n", home.display()));
        script(&home.join(".local/bin/equipe"), "#!/bin/sh\necho \"$*\" > \"$HOME/args\"\ncat > \"$HOME/got\"\necho /far/uploads/kept.mov\n");
        let file = home.join("clip.mov");
        std::fs::write(&file, vec![7u8; 600 << 10]).unwrap();
        let launcher = Launcher { program: fake_ssh, control_dir: home.clone(), askpass: None, extra: vec![] };
        let sent = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let seen = sent.clone();
        let kept = launcher.send_file(&host(), &file, "clip.mov", "video", move |n| seen.store(n, std::sync::atomic::Ordering::Relaxed)).await;
        assert_eq!(kept.ok().as_deref(), Some("/far/uploads/kept.mov"));
        assert_eq!(std::fs::read(home.join("got")).unwrap().len(), 600 << 10, "every byte arrived");
        assert_eq!(std::fs::read_to_string(home.join("args")).unwrap().trim(), format!("receive --kind video --size {} --name clip.mov", 600 << 10));
        assert_eq!(sent.load(std::sync::atomic::Ordering::Relaxed), 600 << 10);
        // A name that could say more to a shell is never sent.
        assert!(launcher.send_file(&host(), &file, "a b;rm.mov", "video", |_| {}).await.is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

    /// The remote login shell runs the line as `$SHELL -c '<line>'`. Run it
    /// through every shell on this machine, with a stand-in `equipe` that only
    /// reports what it got, and a PATH that lacks it except via `~/.cargo/bin`.
    #[test]
    fn the_remote_line_means_the_same_in_every_login_shell() {
        let home = std::env::temp_dir().join(format!("eq-shells-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".cargo/bin")).unwrap();
        let fake = home.join(".cargo/bin/equipe");
        std::fs::write(&fake, "#!/bin/sh\necho \"equipe $*\"\n").unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

        let mut ran = 0;
        for shell in ["/bin/bash", "/bin/zsh", "/bin/sh", "/usr/bin/fish", "/usr/local/bin/fish", "/opt/homebrew/bin/fish", "/bin/dash", "/bin/tcsh", "/bin/csh"] {
            if !Path::new(shell).exists() {
                continue;
            }
            let out = Std::new(shell).arg("-c").arg(attach_command()).env("HOME", &home).env("PATH", "/usr/bin:/bin").output().unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "equipe attach --stdio", "in {shell}: {}", String::from_utf8_lossy(&out.stderr));
            ran += 1;
        }
        assert!(ran >= 2, "at least bash and zsh/sh are here");
        // And a machine without equipe at all ends 127, whichever shell.
        let out = Std::new("/bin/sh").arg("-c").arg(attach_command()).env("HOME", "/nonexistent").env("PATH", "/usr/bin:/bin").output().unwrap();
        assert_eq!(out.status.code(), Some(127));
        let _ = std::fs::remove_dir_all(&home);
    }
}
