//! The app's line to the daemon: the same door, methods and events the terminal
//! console uses (`hello`, `command`, `event` — see `equipe_protocol::rpc`),
//! kept open on a thread of its own.
//!
//! The link connects, reports what it got, forwards events as they arrive, and
//! when the daemon goes away waits and tries again (backing off), so the window
//! never has to know about sockets. It reports only changes: a daemon that stays
//! down is one `Down`, not one per retry, so an idle window draws nothing.

use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_core::gateway::{self, Client};
use equipe_core::protocol::rpc::Hello;
use equipe_core::protocol::{Command, Event};
use equipe_providers::credentials::equipe_home;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

/// Where and as whom to attach.
#[derive(Debug, Clone)]
pub struct Target {
    pub socket: PathBuf,
    pub squad: String,
    /// What the app says it is in `hello`.
    pub version: String,
}

impl Target {
    /// The daemon under `home` (`$EQUIPE_HOME`, or `~/.equipe`), following
    /// `squad` — `$EQUIPE_SQUAD`, else the squad opened last, else the first one.
    pub fn from_env(version: &str) -> Result<Self, String> {
        let home = equipe_home();
        let squad = std::env::var("EQUIPE_SQUAD")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| equipe_core::squads::default_squad(&home))
            .ok_or_else(|| "no squad yet — run `equipe` in a project first".to_owned())?;
        Ok(Self { socket: gateway::socket_path(&home), squad, version: version.to_owned() })
    }
}

/// What the link tells the window.
#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    /// Attached: the daemon's build and the squad's snapshot. After a reconnect
    /// this replaces whatever the window held.
    Attached(Hello),
    /// One event, in order, after the snapshot it follows.
    Event(Event),
    /// Not attached, and why. Retrying in the background.
    Down(String),
}

/// How long to wait between attempts: doubling from `min` up to `max`.
#[derive(Debug, Clone, Copy)]
pub struct Backoff {
    pub min: Duration,
    pub max: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self { min: Duration::from_millis(250), max: Duration::from_secs(5) }
    }
}

/// A daemon that accepts but never answers `hello` counts as down.
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

/// The window's end of the link. Dropping it stops the link's thread.
pub struct Link {
    commands: UnboundedSender<Command>,
}

impl Link {
    /// Send a command to the daemon. Dropped while not attached: what a screen
    /// asks for only makes sense against the snapshot it is looking at.
    #[allow(dead_code)] // the first screens that send commands come next
    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
}

/// Start the link on its own thread. Updates arrive on the receiver; the link
/// stops when either end is dropped.
pub fn spawn(target: Target, backoff: Backoff) -> (Link, UnboundedReceiver<Update>) {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (upd_tx, upd_rx) = mpsc::unbounded_channel();
    std::thread::Builder::new()
        .name("equipe-daemon-link".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a tokio runtime");
            runtime.block_on(run(target, backoff, cmd_rx, upd_tx));
        })
        .expect("spawn the daemon link thread");
    (Link { commands: cmd_tx }, upd_rx)
}

/// Connect, follow, reconnect — until the window goes away.
async fn run(target: Target, backoff: Backoff, mut commands: UnboundedReceiver<Command>, updates: UnboundedSender<Update>) {
    let mut wait = backoff.min;
    let mut down: Option<String> = None;
    loop {
        match attach(&target).await {
            Ok((client, hello)) => {
                wait = backoff.min;
                down = None;
                if updates.send(Update::Attached(hello)).is_err() {
                    return;
                }
                let (to_daemon, mut events) = client.split();
                loop {
                    tokio::select! {
                        event = events.recv() => match event {
                            Some(event) => if updates.send(Update::Event(event)).is_err() { return },
                            None => break,
                        },
                        command = commands.recv() => match command {
                            Some(command) => to_daemon.send(command),
                            None => return,
                        },
                    }
                }
                let reason = "the daemon closed the connection".to_owned();
                if updates.send(Update::Down(reason.clone())).is_err() {
                    return;
                }
                down = Some(reason);
            }
            Err(reason) => {
                // Only a change is news: the same failure again draws nothing.
                if down.as_ref() != Some(&reason) {
                    if updates.send(Update::Down(reason.clone())).is_err() {
                        return;
                    }
                    down = Some(reason);
                }
            }
        }
        // Wait out the backoff; commands sent meanwhile are dropped.
        let sleep = tokio::time::sleep(wait);
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                () = &mut sleep => break,
                command = commands.recv() => if command.is_none() { return },
                () = updates.closed() => return,
            }
        }
        wait = (wait * 2).min(backoff.max);
    }
}

async fn attach(target: &Target) -> Result<(Client, Hello), String> {
    if !target.socket.exists() {
        return Err(not_running(&target.socket));
    }
    match tokio::time::timeout(HELLO_TIMEOUT, Client::connect(&target.socket, &target.version, &target.squad)).await {
        Ok(Ok(attached)) => Ok(attached),
        Ok(Err(err)) if matches!(err.kind(), std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound) => {
            Err(not_running(&target.socket))
        }
        Ok(Err(err)) => Err(err.to_string()),
        Err(_) => Err("the daemon didn't answer".to_owned()),
    }
}

fn not_running(socket: &Path) -> String {
    format!("no daemon at {}", socket.display())
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::Snapshot;
    use equipe_core::protocol::rpc::{self, Message};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;
    use tokio::task::JoinHandle;

    const FAST: Backoff = Backoff { min: Duration::from_millis(20), max: Duration::from_millis(80) };
    const WAIT: Duration = Duration::from_secs(5);

    /// A short path: Unix socket paths are limited to ~104 bytes on macOS.
    fn socket() -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = PathBuf::from(format!("/tmp/eqdl-{}-{}.sock", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_file(&path);
        path
    }

    fn target(socket: &Path) -> Target {
        Target { socket: socket.to_owned(), squad: "acme".into(), version: "test".into() }
    }

    /// A stand-in daemon: answers `hello` with `version`, sends `events` after
    /// it, and reports every command it receives.
    struct Fake {
        task: JoinHandle<()>,
        commands: UnboundedReceiver<Command>,
    }

    impl Fake {
        fn start(socket: &Path, version: &str, events: Vec<Event>) -> Self {
            let listener = UnixListener::bind(socket).unwrap();
            let version = version.to_owned();
            let (cmd_tx, commands) = mpsc::unbounded_channel();
            let task = tokio::spawn(async move {
                loop {
                    let (stream, _) = listener.accept().await.unwrap();
                    let (reader, mut writer) = stream.into_split();
                    let mut lines = BufReader::new(reader).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        let message = Message::from_line(&line).unwrap();
                        match (message.method.as_deref(), message.id) {
                            (Some(rpc::HELLO), Some(id)) => {
                                let params: rpc::HelloParams = message.params_as().unwrap();
                                let snapshot = Snapshot { squad: params.squad, ..Default::default() };
                                let hello = Hello { version: version.clone(), snapshot };
                                writer.write_all(Message::response(id, hello).to_line().as_bytes()).await.unwrap();
                                for event in &events {
                                    writer.write_all(Message::event(event).to_line().as_bytes()).await.unwrap();
                                }
                            }
                            (Some(rpc::COMMAND), None) => {
                                let _ = cmd_tx.send(message.params_as().unwrap());
                            }
                            _ => {}
                        }
                    }
                }
            });
            Self { task, commands }
        }

        /// The daemon dies: the listener and every connection close.
        async fn kill(self, socket: &Path) {
            self.task.abort();
            let _ = self.task.await;
            let _ = std::fs::remove_file(socket);
        }
    }

    async fn next(updates: &mut UnboundedReceiver<Update>) -> Update {
        tokio::time::timeout(WAIT, updates.recv()).await.expect("an update in time").expect("the link is alive")
    }

    fn attached_version(update: Update) -> String {
        match update {
            Update::Attached(hello) => hello.version,
            other => panic!("expected Attached, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn attaches_and_follows_events() {
        let socket = socket();
        let event = Event::WorkEnded { session: 3, ticket: 55 };
        let _fake = Fake::start(&socket, "daemon-1", vec![event.clone()]);
        let (_link, mut updates) = spawn(target(&socket), FAST);

        match next(&mut updates).await {
            Update::Attached(hello) => {
                assert_eq!(hello.version, "daemon-1");
                assert_eq!(hello.snapshot.squad, "acme");
            }
            other => panic!("expected Attached, got {other:?}"),
        }
        assert_eq!(next(&mut updates).await, Update::Event(event));
        let _ = std::fs::remove_file(&socket);
    }

    #[tokio::test]
    async fn commands_reach_the_daemon() {
        let socket = socket();
        let mut fake = Fake::start(&socket, "daemon-1", vec![]);
        let (link, mut updates) = spawn(target(&socket), FAST);
        attached_version(next(&mut updates).await);

        link.send(Command::Interrupt { session: 7 });
        let got = tokio::time::timeout(WAIT, fake.commands.recv()).await.unwrap();
        assert_eq!(got, Some(Command::Interrupt { session: 7 }));
        let _ = std::fs::remove_file(&socket);
    }

    #[tokio::test]
    async fn a_daemon_that_is_off_is_reported_once_and_picked_up_when_it_starts() {
        let socket = socket();
        let (_link, mut updates) = spawn(target(&socket), FAST);
        assert!(matches!(next(&mut updates).await, Update::Down(reason) if reason.contains("no daemon")));

        // Several retries pass while it is off; none of them is news.
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(updates.try_recv().is_err(), "a daemon that stays down must not wake the window");

        let _fake = Fake::start(&socket, "daemon-1", vec![]);
        assert_eq!(attached_version(next(&mut updates).await), "daemon-1");
        let _ = std::fs::remove_file(&socket);
    }

    #[tokio::test]
    async fn reconnects_after_the_daemon_restarts() {
        let socket = socket();
        let fake = Fake::start(&socket, "daemon-1", vec![]);
        let (_link, mut updates) = spawn(target(&socket), FAST);
        assert_eq!(attached_version(next(&mut updates).await), "daemon-1");

        fake.kill(&socket).await;
        assert!(matches!(next(&mut updates).await, Update::Down(_)));

        let _fake = Fake::start(&socket, "daemon-2", vec![]);
        // A retry that lands while the socket is gone may report that first.
        let mut update = next(&mut updates).await;
        while matches!(update, Update::Down(_)) {
            update = next(&mut updates).await;
        }
        assert_eq!(attached_version(update), "daemon-2");
        let _ = std::fs::remove_file(&socket);
    }
}
