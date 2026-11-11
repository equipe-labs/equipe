//! Running the agent-desktop binary: one process per call, as a child of the
//! desktop helper.
//!
//! macOS charges a child process's Accessibility use to the process that
//! launched it (its "responsible" process). The helper is the signed app
//! bundle the person granted, so agent-desktop runs under that one grant: the
//! person never grants agent-desktop separately, and nothing else on the Mac
//! (the daemon, the members) can borrow it.
//!
//! Text a member types goes in on stdin (`--stdin`), never in the arguments,
//! where any process on the Mac could read it. Output is read from stdout only
//! and never logged; stderr is discarded (agent-desktop's diagnostics can name
//! what is on screen).

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use serde_json::Value;

use super::pin;

/// Runs agent-desktop. Behind a trait so the engine is tested without the
/// binary, and never against the real screen.
pub trait Runner: Send + Sync {
    /// Run with `args`, `stdin` piped in when given; the JSON envelope it printed.
    fn run(&self, args: &[String], stdin: Option<&str>, timeout: Duration) -> Result<Value, String>;
}

/// The installed, pinned binary.
pub struct Cli {
    binary: PathBuf,
    /// Its pinned SHA-256; `None` only for tests that run a stand-in.
    sha256: Option<&'static str>,
    state: PathBuf,
    /// Size and modification time when the hash last matched.
    checked: Mutex<Option<(u64, SystemTime)>>,
}

impl Cli {
    /// The pinned release in `home`, if this machine has one. Its state root
    /// is emptied: refs from an earlier run of the helper mean nothing now.
    pub fn installed(home: &Path) -> Option<Self> {
        let archive = pin::this_machine()?;
        let state = pin::state_dir(home);
        let _ = std::fs::remove_dir_all(&state);
        Some(Self { binary: pin::executable(home, archive), sha256: Some(archive.executable_sha256()), state, checked: Mutex::new(None) })
    }

    /// A stand-in binary, not hash-checked: for tests only.
    pub fn unchecked(binary: PathBuf, state: PathBuf) -> Self {
        Self { binary, sha256: None, state, checked: Mutex::new(None) }
    }

    pub fn binary(&self) -> &Path {
        &self.binary
    }

    /// Refuse a binary that isn't the pinned one. Hashed again only when the
    /// file's size or modification time changed since the last match.
    fn verify(&self) -> Result<(), String> {
        let Some(want) = self.sha256 else { return Ok(()) };
        let meta = std::fs::metadata(&self.binary).map_err(|_| NOT_INSTALLED.to_owned())?;
        let stamp = (meta.len(), meta.modified().unwrap_or(SystemTime::UNIX_EPOCH));
        let mut checked = self.checked.lock().unwrap_or_else(|e| e.into_inner());
        if *checked == Some(stamp) {
            return Ok(());
        }
        let got = pin::sha256_file(&self.binary).map_err(|e| e.to_string())?;
        if got != want {
            *checked = None;
            return Err(format!(
                "The installed agent-desktop isn't the release equipe pins ({}): it won't run. Ask the person to run `equipe desktop setup` again.",
                pin::VERSION
            ));
        }
        *checked = Some(stamp);
        Ok(())
    }
}

/// What a member is told when agent-desktop isn't on the machine yet.
pub const NOT_INSTALLED: &str = "The accessibility engine (agent-desktop) isn't installed on this machine yet. Ask the person to run `equipe desktop setup`. Meanwhile the screenshot tools still work.";

impl Runner for Cli {
    fn run(&self, args: &[String], stdin: Option<&str>, timeout: Duration) -> Result<Value, String> {
        self.verify()?;
        std::fs::create_dir_all(&self.state).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = std::fs::set_permissions(&self.state, std::fs::Permissions::from_mode(0o700));
        }
        let mut child = Command::new(&self.binary)
            .args(args)
            .env("AGENT_DESKTOP_HOME", &self.state)
            .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("couldn't start agent-desktop: {e}"))?;
        if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
            // Dropped at the end of the block: agent-desktop reads to the end.
            let _ = pipe.write_all(text.as_bytes());
        }
        let mut stdout = child.stdout.take().ok_or("no output from agent-desktop")?;
        let reader = std::thread::spawn(move || {
            let mut out = Vec::new();
            let _ = stdout.read_to_end(&mut out);
            out
        });
        let deadline = Instant::now() + timeout;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("agent-desktop didn't answer within {} s; it was stopped.", timeout.as_secs()));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(15)),
                Err(err) => return Err(err.to_string()),
            }
        }
        let out = reader.join().unwrap_or_default();
        envelope(&out)
    }
}

/// The JSON envelope in agent-desktop's output: its last JSON line.
pub fn envelope(out: &[u8]) -> Result<Value, String> {
    let text = String::from_utf8_lossy(out);
    text.lines()
        .rev()
        .map(str::trim)
        .filter(|l| l.starts_with('{'))
        .find_map(|l| serde_json::from_str::<Value>(l).ok())
        .or_else(|| serde_json::from_str::<Value>(text.trim()).ok())
        .ok_or_else(|| "agent-desktop's answer was unreadable".to_owned())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A stand-in agent-desktop: prints its arguments and stdin back as an envelope.
    fn echo(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let path = dir.join("agent-desktop");
        std::fs::write(
            &path,
            "#!/bin/sh\nIN=\"\"\nif [ \"$1\" = \"--stdin-test\" ]; then IN=$(cat); fi\nprintf '{\"ok\":true,\"command\":\"echo\",\"data\":{\"args\":\"%s\",\"stdin\":\"%s\",\"home\":\"%s\"}}\\n' \"$*\" \"$IN\" \"$AGENT_DESKTOP_HOME\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("eq-adcli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn text_goes_in_on_stdin_and_the_state_root_is_private() {
        let dir = scratch("stdin");
        let cli = Cli::unchecked(echo(&dir), dir.join("state"));
        let out = cli.run(&["--stdin-test".into(), "type".into()], Some("secret words"), Duration::from_secs(10)).unwrap();
        assert_eq!(out["data"]["stdin"], "secret words");
        assert!(!out["data"]["args"].as_str().unwrap().contains("secret"), "never in the arguments");
        assert_eq!(out["data"]["home"], dir.join("state").display().to_string());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_binary_that_isnt_the_pinned_one_never_runs() {
        let dir = scratch("pin");
        let cli = Cli { binary: echo(&dir), sha256: Some("0000"), state: dir.join("state"), checked: Mutex::new(None) };
        let refused = cli.run(&["version".into()], None, Duration::from_secs(5)).unwrap_err();
        assert!(refused.contains("isn't the release equipe pins"), "{refused}");
        let missing = Cli { binary: dir.join("nowhere"), sha256: Some("0000"), state: dir.join("state"), checked: Mutex::new(None) };
        assert_eq!(missing.run(&[], None, Duration::from_secs(5)).unwrap_err(), NOT_INSTALLED);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_call_that_hangs_is_stopped() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = scratch("hang");
        let path = dir.join("agent-desktop");
        std::fs::write(&path, "#!/bin/sh\nsleep 30\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let cli = Cli::unchecked(path, dir.join("state"));
        let started = Instant::now();
        let refused = cli.run(&[], None, Duration::from_millis(300)).unwrap_err();
        assert!(refused.contains("didn't answer"), "{refused}");
        assert!(started.elapsed() < Duration::from_secs(5));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_envelope_is_the_last_json_line() {
        let out = b"warming up\n{\"ok\":false}\n{\"ok\":true,\"command\":\"version\"}\n";
        assert_eq!(envelope(out).unwrap()["command"], "version");
        assert!(envelope(b"nothing").is_err());
    }
}
