//! `equipe attach --stdio`, the way a screen on another machine reaches the
//! daemon: `ssh <host> equipe attach --stdio`. The remote shell is played by
//! `sh -c`, so the bytes take the same path SSH gives them (a remote command's
//! stdin and stdout). Set `EQUIPE_TEST_SSH=<host>` (e.g. `localhost`) to go
//! through a real `ssh` too; the host must reach this build at the same path.

#![cfg(unix)]

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use equipe_core::protocol::rpc::{self, Attached, Hello, HelloParams, Message};
use equipe_core::protocol::{Command as Cmd, Event, Mode};

const BIN: &str = env!("CARGO_BIN_EXE_equipe");

/// A home of its own under /tmp (socket paths are limited to ~100 bytes), with
/// the squad `acme`. Stops the daemon it started when dropped.
struct Home(PathBuf);

impl Home {
    fn new(name: &str) -> Self {
        let dir = PathBuf::from("/tmp").join(format!("eqa-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let project = dir.join("proj");
        std::fs::create_dir_all(&project).unwrap();
        let home = Self(dir);
        equipe_core::squads::create(&home.equipe(), "acme", &project.canonicalize().unwrap()).unwrap();
        home
    }

    fn equipe(&self) -> PathBuf {
        self.0.join(".equipe")
    }

    /// `equipe <args>` as run on the daemon's machine, through its shell — or
    /// through `ssh` when `EQUIPE_TEST_SSH` names a host.
    fn equipe_cmd(&self, args: &str) -> Command {
        let env = format!("HOME='{}' EQUIPE_HOME='{}'", self.0.display(), self.equipe().display());
        match std::env::var("EQUIPE_TEST_SSH") {
            Ok(host) => {
                let mut ssh = Command::new("ssh");
                ssh.args(["-T", "-o", "BatchMode=yes", &host, &format!("{env} exec '{BIN}' {args}")]);
                ssh
            }
            Err(_) => {
                let mut sh = Command::new("sh");
                sh.args(["-c", &format!("{env} exec \"$0\" {args}"), BIN]);
                sh
            }
        }
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = self.equipe_cmd("daemon stop").stdout(Stdio::null()).stderr(Stdio::null()).status();
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A running bridge: lines go in on its stdin and come back from its stdout.
struct Bridge {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
}

impl Bridge {
    fn start(home: &Home, args: &str) -> Self {
        let mut child = home
            .equipe_cmd(&format!("attach --stdio {args}"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let stdin = child.stdin.take();
        Self { child, stdin, lines }
    }

    fn send(&mut self, message: &Message) {
        let stdin = self.stdin.as_mut().unwrap();
        stdin.write_all(message.to_line().as_bytes()).unwrap();
        stdin.flush().unwrap();
    }

    /// The next line that `wanted` accepts, skipping others.
    fn until(&self, wanted: impl Fn(&Message) -> bool) -> Message {
        loop {
            let line = self.lines.recv_timeout(Duration::from_secs(15)).expect("a line from the bridge in time");
            let message = Message::from_line(&line).unwrap_or_else(|e| panic!("not a protocol line ({e}): {line}"));
            if wanted(&message) {
                return message;
            }
        }
    }

    /// Hang up, as SSH does when the screen goes, and wait for the bridge to end.
    fn hang_up(mut self) -> std::process::ExitStatus {
        drop(self.stdin.take());
        wait(&mut self.child)
    }
}

fn wait(child: &mut Child) -> std::process::ExitStatus {
    for _ in 0..300 {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    panic!("the bridge didn't end");
}

fn attached(bridge: &Bridge) -> Attached {
    let first = bridge.until(|_| true);
    assert_eq!(first.method.as_deref(), Some(rpc::ATTACHED), "the first line says who answered: {first:?}");
    first.params_as::<Attached>().expect("attached params")
}

#[test]
fn a_screen_over_ssh_says_hello_gets_the_snapshot_and_follows_events() {
    let home = Home::new("flow");

    // No daemon yet: the bridge starts one, as the console does.
    let mut bridge = Bridge::start(&home, "");
    let first = attached(&bridge);
    assert!(first.started, "{first:?}");
    assert_eq!(first.protocol, rpc::PROTOCOL);
    assert_eq!(first.daemon.as_deref(), Some(first.bridge.as_str()), "the daemon it started is this build");

    bridge.send(&Message::request(1, rpc::HELLO, HelloParams { version: "remote-screen".into(), squad: "acme".into() }));
    let hello = bridge.until(|m| m.id == Some(1)).result_as::<Hello>().expect("a hello back");
    assert_eq!(hello.snapshot.squad, "acme");
    assert_eq!(hello.version, first.bridge);

    bridge.send(&Message::command(&Cmd::SetMode { squad: "acme".into(), member: None, mode: Some(Mode::Ask) }));
    let event = bridge.until(|m| m.method.as_deref() == Some(rpc::EVENT));
    assert_eq!(
        event.params_as::<Event>(),
        Some(Event::ModeChanged { squad: "acme".into(), member: None, mode: Some(Mode::Ask) })
    );

    // Other requests pass through untouched, ids and all.
    bridge.send(&Message::request(7, rpc::VERSION, ()));
    let version = bridge.until(|m| m.id == Some(7)).result_as::<rpc::Version>().expect("a version back");
    assert_eq!(version.protocol, rpc::PROTOCOL);

    // What a screen on another machine can't read from the disk, it asks for.
    bridge.send(&Message::request(8, rpc::SQUADS, ()));
    let squads = bridge.until(|m| m.id == Some(8)).result_as::<rpc::Squads>().expect("squads back");
    assert_eq!(squads.last.as_deref(), Some("acme"));
    assert_eq!(squads.squads.len(), 1);
    assert!(squads.squads[0].home.ends_with("squads/acme/home"), "{squads:?}");
    assert_eq!(squads.squads[0].projects, vec![home.0.join("proj").canonicalize().unwrap()]);

    // An image goes up as base64 and comes back as a path on the daemon's machine.
    bridge.send(&Message::request(9, rpc::UPLOAD, rpc::Upload { name: "../../shot.PNG".into(), data: "iVBORw0KGgo=".into() }));
    let up = bridge.until(|m| m.id == Some(9)).result_as::<rpc::Uploaded>().expect("an upload path");
    assert_eq!(up.path.parent(), Some(home.equipe().join("uploads").as_path()), "named by equipe, not the screen");
    assert_eq!(up.path.extension().unwrap(), "png");
    assert_eq!(std::fs::read(&up.path).unwrap(), b"\x89PNG\r\n\x1a\n");
    bridge.send(&Message::request(10, rpc::UPLOAD, rpc::Upload { name: "notes.sh".into(), data: "ZWNobw==".into() }));
    assert!(bridge.until(|m| m.id == Some(10)).error.is_some(), "only images");

    assert!(bridge.hang_up().success(), "the screen leaving ends the bridge cleanly");

    // A second screen finds the daemon up, and the mode the first one set.
    let mut again = Bridge::start(&home, "--no-start");
    assert!(!attached(&again).started);
    again.send(&Message::request(1, rpc::HELLO, HelloParams { version: "remote-screen".into(), squad: "acme".into() }));
    assert!(again.until(|m| m.id == Some(1)).result_as::<Hello>().is_some());
    assert!(again.hang_up().success());
}

#[test]
fn the_daemon_going_away_ends_the_bridge() {
    let home = Home::new("stop");
    let bridge = Bridge::start(&home, "");
    attached(&bridge);
    assert!(home.equipe_cmd("daemon stop").stdout(Stdio::null()).status().unwrap().success());
    let mut child = bridge.child;
    assert!(wait(&mut child).success(), "a stopped daemon ends the stream, not an error");
}

#[test]
fn with_no_start_and_no_daemon_it_says_so_and_prints_nothing() {
    let home = Home::new("off");
    let out = home.equipe_cmd("attach --stdio --no-start").stdin(Stdio::null()).output().unwrap();
    assert_eq!(out.status.code(), Some(69), "EX_UNAVAILABLE: {out:?}");
    assert!(out.stdout.is_empty(), "stdout carries only the protocol");
    assert!(String::from_utf8_lossy(&out.stderr).contains("daemon isn't running"));
    assert!(!Path::new(&home.equipe().join("equipe.sock")).exists());
}
