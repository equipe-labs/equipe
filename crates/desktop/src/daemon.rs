//! The app's line to the daemon: the same door, methods and events the terminal
//! console uses (`hello`, `command`, `event` — see `equipe_protocol::rpc`),
//! kept open on a thread of its own.
//!
//! The daemon may be on this Mac (its socket) or on another machine, reached
//! with the system `ssh` running `equipe attach --stdio` (docs/DESKTOP.md,
//! "Remote"). Both are the same protocol over a pair of byte streams
//! (`wire.rs`), so everything past the connect is one code path.
//!
//! The link connects, reports what it got, forwards events as they arrive, and
//! when the daemon goes away waits and tries again (backing off), so the window
//! never has to know about sockets or processes. It reports only changes: a
//! daemon that stays down is one `Down`, not one per retry, so an idle window
//! draws nothing. A cause only the person can fix (a refused password, a changed
//! host key) is not retried on a timer; the window offers "try again". A Mac
//! that wakes from sleep drops its SSH connection and starts over at once.

use equipe_core::t;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use equipe_core::protocol::rpc::{Attached, Hello, Squads};
use equipe_core::protocol::{Command, Event};
use tokio::net::UnixStream;
use tokio::sync::Notify;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::askpass::{self, Prompt};
use crate::failure::Failure;
use crate::ssh::{self, Launcher, Process};
use crate::target::Endpoint;
use crate::wire::{self, Opts, Stop, Wire};

/// Where and as whom to attach.
#[derive(Debug, Clone)]
pub struct Target {
    pub endpoint: Endpoint,
    /// The squad to follow when it exists; else the one opened last, else the
    /// first (the daemon says which, in `squads`).
    pub prefer: Option<String>,
    /// What the app says it is in `hello`.
    pub version: String,
    /// How to run `ssh`; `None`: the system's, with questions asked in the window.
    pub ssh: Option<Launcher>,
}

impl Target {
    pub fn new(endpoint: Endpoint, version: &str) -> Self {
        let prefer = std::env::var("EQUIPE_SQUAD").ok().filter(|s| !s.is_empty());
        Self { endpoint, prefer, version: version.to_owned(), ssh: None }
    }
}

/// What the link tells the window.
#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    /// The squads on the daemon's machine; before each `Attached`.
    Squads(Squads),
    /// What the SSH bridge said first: which equipe and daemon answered, and
    /// whether it had to start the daemon.
    Bridge(Attached),
    /// Attached: the daemon's build and the squad's snapshot. After a reconnect
    /// this replaces whatever the window held.
    Attached(Hello),
    /// One event, in order, after the snapshot it follows.
    Event(Event),
    /// Not attached, and why. Retrying in the background — unless the cause is
    /// one the person has to fix.
    Down(Failure),
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

impl Backoff {
    /// Over a network a try costs more, and the other end is often just asleep.
    pub fn remote() -> Self {
        Self { min: Duration::from_millis(500), max: Duration::from_secs(10) }
    }
}

/// A daemon that accepts but never answers `hello` counts as down.
const STEP_TIMEOUT: Duration = Duration::from_secs(5);
/// Over SSH the first line may wait for a person typing a password.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);
/// How often the clock is looked at, to notice the Mac slept.
const TICK: Duration = Duration::from_secs(2);
/// A gap this much longer than a tick means the machine was asleep.
const SLEPT: Duration = Duration::from_secs(10);

/// The window's end of the link. Dropping it stops the link's thread.
pub struct Link {
    commands: UnboundedSender<Command>,
    retry: Arc<Notify>,
    /// The live connection's way to ask the daemon things, while there is one.
    asking: Asking,
}

/// The current connection's request door; empty while not attached.
type Asking = Arc<std::sync::Mutex<Option<wire::Commands>>>;

/// Keeps images on the daemon's machine, from any thread.
#[derive(Clone)]
pub struct Uploader(Asking);

impl Uploader {
    /// Send `bytes` to the daemon and wait for where it kept them. Blocks:
    /// call it off the UI thread.
    pub fn upload(&self, name: &str, bytes: &[u8], kind: Option<equipe_core::protocol::rpc::UploadKind>) -> Result<std::path::PathBuf, String> {
        let door = self
            .0
            .lock()
            .unwrap()
            .clone()
            .ok_or(t!("daemon.not_connected"))?;
        match door
            .upload(name, bytes, kind)
            .recv_timeout(Duration::from_secs(120))
        {
            Ok(answer) => answer.and_then(|v| serde_json::from_value::<equipe_core::protocol::rpc::Uploaded>(v).map(|u| u.path).map_err(|_| t!("failure.answer_unreadable").into())),
            Err(_) => Err(t!("daemon.upload_no_answer").into()),
        }
    }

    /// A file a message attached, read back from the daemon's machine (its
    /// size only, when `size_only`). Blocks: call it off the UI thread.
    pub fn read(&self, path: &str, size_only: bool) -> Result<equipe_core::protocol::rpc::UploadRead, String> {
        let door = self.0.lock().unwrap().clone().ok_or(t!("daemon.not_connected"))?;
        match door.read_upload(path, size_only).recv_timeout(Duration::from_secs(60)) {
            Ok(answer) => answer.and_then(|v| serde_json::from_value(v).map_err(|_| t!("failure.answer_unreadable").into())),
            Err(_) => Err(t!("daemon.upload_no_answer").into()),
        }
    }
}

impl Uploader {
    /// A member's picture, from the daemon — wherever it runs; it gives only
    /// what `squad` shows. Blocks: call it off the UI thread.
    pub fn read_image(&self, squad: &str, path: &str, max: Option<u32>) -> Result<equipe_core::protocol::rpc::ImageRead, String> {
        let door = self.0.lock().unwrap().clone().ok_or(t!("daemon.not_connected"))?;
        match door.read_image(squad, path, max).recv_timeout(Duration::from_secs(60)) {
            Ok(answer) => answer.and_then(|v| serde_json::from_value(v).map_err(|_| t!("failure.answer_unreadable").into())),
            Err(_) => Err(t!("daemon.upload_no_answer").into()),
        }
    }

    /// The pictures in a folder a reply names, from the daemon — only one
    /// `squad`'s conversation names. Blocks: call it off the UI thread.
    pub fn list_images(&self, squad: &str, folder: &str) -> Result<equipe_core::protocol::rpc::ImageList, String> {
        let door = self.0.lock().unwrap().clone().ok_or(t!("daemon.not_connected"))?;
        match door.list_images(squad, folder).recv_timeout(Duration::from_secs(60)) {
            Ok(answer) => answer.and_then(|v| serde_json::from_value(v).map_err(|_| t!("failure.answer_unreadable").into())),
            Err(_) => Err(t!("daemon.upload_no_answer").into()),
        }
    }
}

impl Link {
    /// Send a command to the daemon. Dropped while not attached: what a screen
    /// asks for only makes sense against the snapshot it is looking at.
    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    pub fn uploader(&self) -> Uploader {
        Uploader(self.asking.clone())
    }

    /// Try again now (after the person fixed what was wrong).
    pub fn retry(&self) {
        self.retry.notify_one();
    }
}

/// What `spawn` hands back.
pub struct Spawned {
    pub link: Link,
    pub updates: UnboundedReceiver<Update>,
    /// Questions `ssh` asks the person (passwords, host keys).
    pub prompts: UnboundedReceiver<Prompt>,
}

/// Start the link on its own thread. Updates arrive on the receiver; the link
/// stops when either end is dropped.
pub fn spawn(target: Target, backoff: Backoff) -> Spawned {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (upd_tx, upd_rx) = mpsc::unbounded_channel();
    let (prompt_tx, prompt_rx) = mpsc::unbounded_channel();
    let retry = Arc::new(Notify::new());
    let again = retry.clone();
    let asking = Asking::default();
    let door = asking.clone();
    std::thread::Builder::new()
        .name("equipe-daemon-link".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a tokio runtime");
            runtime.block_on(run(target, backoff, cmd_rx, upd_tx, again, prompt_tx, door));
        })
        .expect("spawn the daemon link thread");
    Spawned {
        link: Link {
            commands: cmd_tx,
            retry,
            asking,
        },
        updates: upd_rx,
        prompts: prompt_rx,
    }
}

/// A live connection.
struct Connected {
    greeting: wire::Greeting,
    wire: Wire,
    /// The `ssh` behind it, when remote.
    process: Option<Process>,
}

/// What `ssh` needs from the app: the launcher, and the box that answers its questions.
struct SshSide {
    launcher: Launcher,
    host: crate::target::SshHost,
    server: Option<askpass::Server>,
}

/// Notices that the machine slept: the wall clock jumps while ticks don't.
struct Sleep {
    last: SystemTime,
}

impl Sleep {
    fn new() -> Self {
        Self { last: SystemTime::now() }
    }

    /// Called on each tick: whether more time passed than a tick explains.
    fn woke(&mut self) -> bool {
        self.woke_at(SystemTime::now())
    }

    fn woke_at(&mut self, now: SystemTime) -> bool {
        let gap = now.duration_since(self.last).unwrap_or_default();
        self.last = now;
        gap > SLEPT
    }
}

enum Followed {
    /// The connection ended (or was cut on purpose after waking).
    Ended(Failure),
    /// The window is gone.
    Window,
}

/// Connect, follow, reconnect — until the window goes away.
async fn run(
    target: Target,
    backoff: Backoff,
    mut commands: UnboundedReceiver<Command>,
    updates: UnboundedSender<Update>,
    retry: Arc<Notify>,
    prompts: UnboundedSender<Prompt>,
    asking: Asking,
) {
    let side = match &target.endpoint {
        Endpoint::Ssh(host) => {
            let (launcher, server) = match target.ssh.clone() {
                Some(launcher) => (launcher, None),
                None => system_ssh(&prompts),
            };
            Some(SshSide { launcher, host: host.clone(), server })
        }
        Endpoint::Local { .. } => None,
    };
    let mut wait = backoff.min;
    let mut down: Option<Failure> = None;
    let mut sleep = Sleep::new();
    loop {
        if let Some(server) = side.as_ref().and_then(|s| s.server.as_ref()) {
            server.forget_cancelled();
        }
        let failure = match attach(&target, side.as_ref()).await {
            Ok(mut conn) => {
                wait = backoff.min;
                if let Some(bridge) = conn.greeting.bridge.clone()
                    && updates.send(Update::Bridge(bridge)).is_err()
                {
                    return;
                }
                if updates.send(Update::Squads(conn.greeting.squads.clone())).is_err() || updates.send(Update::Attached(conn.greeting.hello.clone())).is_err() {
                    return;
                }
                *asking.lock().unwrap() = Some(conn.wire.commands.clone());
                let followed = follow(
                    &mut conn,
                    &mut commands,
                    &updates,
                    &mut sleep,
                    side.as_ref(),
                )
                .await;
                *asking.lock().unwrap() = None;
                match followed {
                    Followed::Window => return,
                    Followed::Ended(failure) => {
                        if updates.send(Update::Down(failure.clone())).is_err() {
                            return;
                        }
                        down = Some(failure.clone());
                        failure
                    }
                }
            }
            Err(failure) => {
                // Only a change is news: the same failure again draws nothing.
                if down.as_ref() != Some(&failure) {
                    if updates.send(Update::Down(failure.clone())).is_err() {
                        return;
                    }
                    down = Some(failure.clone());
                }
                failure
            }
        };
        if failure.waits_for_person() {
            // Nothing to gain from another go until the person says so.
            loop {
                tokio::select! {
                    () = retry.notified() => break,
                    command = commands.recv() => if command.is_none() { return },
                    () = updates.closed() => return,
                }
            }
            wait = backoff.min;
            continue;
        }
        // Wait out the backoff; commands sent meanwhile are dropped. A try-again
        // click or waking from sleep cuts it short.
        let nap = tokio::time::sleep(wait);
        tokio::pin!(nap);
        let mut tick = tokio::time::interval(TICK);
        let mut woke = false;
        loop {
            tokio::select! {
                () = &mut nap => break,
                () = retry.notified() => { wait = backoff.min; break },
                _ = tick.tick() => if sleep.woke() && side.is_some() { woke = true; break },
                command = commands.recv() => if command.is_none() { return },
                () = updates.closed() => return,
            }
        }
        if woke {
            if let Some(side) = &side {
                side.launcher.drop_shared_connection(&side.host).await;
            }
            wait = backoff.min;
        } else {
            wait = (wait * 2).min(backoff.max);
        }
    }
}

/// The system `ssh`, with its questions routed to the window.
fn system_ssh(prompts: &UnboundedSender<Prompt>) -> (Launcher, Option<askpass::Server>) {
    let started = askpass::private_dir().and_then(|dir| askpass::Server::start(&dir));
    match (started, std::env::current_exe()) {
        (Ok((server, mut questions)), Ok(exe)) => {
            let launcher = Launcher::system(Some(ssh::Askpass { exe, socket: server.socket.clone(), token: server.token.clone() }));
            let prompts = prompts.clone();
            tokio::spawn(async move {
                while let Some(question) = questions.recv().await {
                    if prompts.send(question).is_err() {
                        break;
                    }
                }
            });
            (launcher, Some(server))
        }
        // Without a place for the helper's socket ssh can still use keys and
        // the agent; a password has nowhere to be typed and fails as such.
        _ => (Launcher::system(None), None),
    }
}

/// Pass events up and commands down until the connection ends.
async fn follow(
    conn: &mut Connected,
    commands: &mut UnboundedReceiver<Command>,
    updates: &UnboundedSender<Update>,
    sleep: &mut Sleep,
    side: Option<&SshSide>,
) -> Followed {
    let mut tick = tokio::time::interval(TICK);
    loop {
        tokio::select! {
            event = conn.wire.events.recv() => match event {
                Some(event) => if updates.send(Update::Event(event)).is_err() { return Followed::Window },
                None => break,
            },
            command = commands.recv() => match command {
                Some(command) => conn.wire.commands.send(command),
                None => return Followed::Window,
            },
            _ = tick.tick() => if sleep.woke() && let Some(side) = side {
                // The Mac slept: the TCP connection under SSH is very likely
                // dead and would take a minute to say so. Start over now.
                if let Some(process) = &mut conn.process {
                    process.kill().await;
                }
                side.launcher.drop_shared_connection(&side.host).await;
                return Followed::Ended(Failure::Closed);
            },
        }
    }
    let failure = match &mut conn.process {
        Some(process) => match process.finish(false).await {
            // It was working a moment ago: whatever it says now, we simply lost it.
            Failure::Other(_) => Failure::Closed,
            failure => failure,
        },
        None => Failure::Closed,
    };
    Followed::Ended(failure)
}

async fn attach(target: &Target, side: Option<&SshSide>) -> Result<Connected, Failure> {
    let opts = |bridged| Opts {
        version: target.version.clone(),
        prefer: target.prefer.clone(),
        bridged,
        first: if bridged { SIGN_IN_TIMEOUT } else { STEP_TIMEOUT },
        step: STEP_TIMEOUT,
    };
    match (&target.endpoint, side) {
        (Endpoint::Local { socket }, _) => {
            // Off on this Mac: started (once per launch), with the app's own
            // `equipe` when the Mac has none — then looked for again.
            #[cfg(not(test))]
            if !socket.exists() && tokio::task::spawn_blocking(crate::install::wake_daemon).await.unwrap_or(false) {
                for _ in 0..40 {
                    if socket.exists() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
            if !socket.exists() {
                return Err(not_running(socket));
            }
            let stream = match UnixStream::connect(socket).await {
                Ok(stream) => stream,
                Err(err) if matches!(err.kind(), std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound) => return Err(not_running(socket)),
                Err(err) => return Err(Failure::Other(err.to_string())),
            };
            let (reader, writer) = stream.into_split();
            match wire::connect(reader, writer, &opts(false)).await {
                Ok((greeting, wire)) => Ok(Connected { greeting, wire, process: None }),
                Err(Stop::Eof) => Err(Failure::Closed),
                Err(Stop::Failed(failure)) => Err(failure),
            }
        }
        (Endpoint::Ssh(_), Some(side)) => {
            let (mut process, stdout, stdin) = side.launcher.attach(&side.host).map_err(|e| Failure::Other(format!("ssh: {e}")))?;
            match wire::connect(stdout, stdin, &opts(true)).await {
                Ok((greeting, wire)) => Ok(Connected { greeting, wire, process: Some(process) }),
                Err(Stop::Eof) => {
                    let cancelled = side.server.as_ref().is_some_and(askpass::Server::take_cancelled);
                    Err(process.finish(cancelled).await)
                }
                Err(Stop::Failed(failure)) => {
                    process.kill().await;
                    Err(failure)
                }
            }
        }
        (Endpoint::Ssh(_), None) => Err(Failure::Other(t!("failure.ssh_not_set_up").into())),
    }
}

fn not_running(socket: &std::path::Path) -> Failure {
    Failure::LocalDaemonOff { socket: socket.display().to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::Snapshot;
    use equipe_core::protocol::rpc::{self, Message, SquadEntry};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;
    use tokio::task::JoinHandle;

    const FAST: Backoff = Backoff { min: Duration::from_millis(20), max: Duration::from_millis(80) };
    const WAIT: Duration = Duration::from_secs(5);

    static N: AtomicUsize = AtomicUsize::new(0);

    /// A short path: Unix socket paths are limited to ~104 bytes on macOS.
    fn socket() -> PathBuf {
        let path = PathBuf::from(format!("/tmp/eqdl-{}-{}.sock", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_file(&path);
        path
    }

    fn target(socket: &Path) -> Target {
        Target { endpoint: Endpoint::Local { socket: socket.to_owned() }, prefer: Some("acme".into()), version: "test".into(), ssh: None }
    }

    fn squads() -> Squads {
        Squads { squads: vec![SquadEntry { name: "acme".into(), projects: vec![], home: "/h/acme".into() }], last: Some("acme".into()) }
    }

    fn hello(version: &str) -> Hello {
        Hello { version: version.into(), snapshot: Snapshot { squad: "acme".into(), ..Default::default() } }
    }

    /// A stand-in daemon: answers `squads` and `hello` (with `version`), sends
    /// `events` after it, and reports every command it receives.
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
                            (Some(rpc::SQUADS), Some(id)) => {
                                writer.write_all(Message::response(id, squads()).to_line().as_bytes()).await.unwrap();
                            }
                            (Some(rpc::HELLO), Some(id)) => {
                                writer.write_all(Message::response(id, hello(&version)).to_line().as_bytes()).await.unwrap();
                                for event in &events {
                                    writer.write_all(Message::event(event).to_line().as_bytes()).await.unwrap();
                                }
                            }
                            (Some(rpc::COMMAND), None) => {
                                let _ = cmd_tx.send(message.params_as().unwrap());
                            }
                            (Some(rpc::UPLOAD), Some(id)) => {
                                let upload: rpc::Upload = message.params_as().unwrap();
                                let reply = if upload.name.ends_with(".png") {
                                    Message::response(
                                        id,
                                        rpc::Uploaded {
                                            path: format!("/up/{}", upload.data).into(),
                                        },
                                    )
                                } else {
                                    Message::failure(id, -32000, "only images can be uploaded")
                                };
                                writer.write_all(reply.to_line().as_bytes()).await.unwrap();
                            }
                            (Some(rpc::READ_UPLOAD), Some(id)) => {
                                let ask: rpc::ReadUpload = message.params_as().unwrap();
                                let reply = if ask.path.starts_with("/up/") {
                                    Message::response(id, rpc::UploadRead { size: 2, data: (!ask.size_only).then(|| "aGk=".to_owned()) })
                                } else {
                                    Message::failure(id, -32000, "not an upload")
                                };
                                writer.write_all(reply.to_line().as_bytes()).await.unwrap();
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

    /// The next update that isn't the squad list or the bridge's greeting.
    async fn next_main(updates: &mut UnboundedReceiver<Update>) -> Update {
        loop {
            match next(updates).await {
                Update::Squads(_) | Update::Bridge(_) => {}
                other => return other,
            }
        }
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
        let mut spawned = spawn(target(&socket), FAST);

        // The squad list comes first, from the daemon: no file on this disk is read.
        assert_eq!(next(&mut spawned.updates).await, Update::Squads(squads()));
        match next(&mut spawned.updates).await {
            Update::Attached(hello) => {
                assert_eq!(hello.version, "daemon-1");
                assert_eq!(hello.snapshot.squad, "acme");
            }
            other => panic!("expected Attached, got {other:?}"),
        }
        assert_eq!(next(&mut spawned.updates).await, Update::Event(event));
        let _ = std::fs::remove_file(&socket);
    }

    #[tokio::test]
    async fn an_image_is_kept_by_the_daemon_and_its_path_comes_back() {
        let socket = socket();
        let _fake = Fake::start(&socket, "daemon-1", vec![]);
        let mut spawned = spawn(target(&socket), FAST);
        attached_version(next_main(&mut spawned.updates).await);

        let uploader = spawned.link.uploader();
        let asked = tokio::task::spawn_blocking(move || {
            (
                uploader.upload("a.png", b"hi", None),
                uploader.upload("a.txt", b"hi", None),
            )
        })
        .await
        .unwrap();
        // The fake answers with the bytes' base64 in the path.
        assert_eq!(asked.0, Ok(std::path::PathBuf::from("/up/aGk=")));
        assert_eq!(asked.1, Err("only images can be uploaded".to_owned()));
        let _ = std::fs::remove_file(&socket);
    }

    #[tokio::test]
    async fn an_attachment_is_read_back_from_the_daemons_uploads() {
        let socket = socket();
        let _fake = Fake::start(&socket, "daemon-1", vec![]);
        let mut spawned = spawn(target(&socket), FAST);
        attached_version(next_main(&mut spawned.updates).await);
        let uploader = spawned.link.uploader();
        let asked = tokio::task::spawn_blocking(move || (uploader.read("/up/a.png", false), uploader.read("/up/a.pdf", true), uploader.read("/etc/passwd", false)))
            .await
            .unwrap();
        assert_eq!(asked.0, Ok(rpc::UploadRead { size: 2, data: Some("aGk=".into()) }));
        assert_eq!(asked.1, Ok(rpc::UploadRead { size: 2, data: None }));
        assert_eq!(asked.2, Err("not an upload".to_owned()));
        let _ = std::fs::remove_file(&socket);
    }

    #[tokio::test]
    async fn an_image_cannot_be_kept_while_the_daemon_is_off() {
        let socket = socket();
        let spawned = spawn(target(&socket), FAST);
        let uploader = spawned.link.uploader();
        let asked = tokio::task::spawn_blocking(move || uploader.upload("a.png", b"hi", None))
            .await
            .unwrap();
        assert!(asked.is_err());
    }

    #[tokio::test]
    async fn commands_reach_the_daemon() {
        let socket = socket();
        let mut fake = Fake::start(&socket, "daemon-1", vec![]);
        let mut spawned = spawn(target(&socket), FAST);
        attached_version(next_main(&mut spawned.updates).await);

        spawned.link.send(Command::Interrupt { session: 7 });
        let got = tokio::time::timeout(WAIT, fake.commands.recv()).await.unwrap();
        assert_eq!(got, Some(Command::Interrupt { session: 7 }));
        let _ = std::fs::remove_file(&socket);
    }

    #[tokio::test]
    async fn a_daemon_that_is_off_is_reported_once_and_picked_up_when_it_starts() {
        let socket = socket();
        let mut spawned = spawn(target(&socket), FAST);
        assert!(matches!(next(&mut spawned.updates).await, Update::Down(Failure::LocalDaemonOff { .. })));

        // Several retries pass while it is off; none of them is news.
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(spawned.updates.try_recv().is_err(), "a daemon that stays down must not wake the window");

        let _fake = Fake::start(&socket, "daemon-1", vec![]);
        assert_eq!(attached_version(next_main(&mut spawned.updates).await), "daemon-1");
        let _ = std::fs::remove_file(&socket);
    }

    #[tokio::test]
    async fn reconnects_after_the_daemon_restarts() {
        let socket = socket();
        let fake = Fake::start(&socket, "daemon-1", vec![]);
        let mut spawned = spawn(target(&socket), FAST);
        assert_eq!(attached_version(next_main(&mut spawned.updates).await), "daemon-1");

        fake.kill(&socket).await;
        assert!(matches!(next_main(&mut spawned.updates).await, Update::Down(_)));

        let _fake = Fake::start(&socket, "daemon-2", vec![]);
        // A retry that lands while the socket is gone may report that first.
        let mut update = next_main(&mut spawned.updates).await;
        while matches!(update, Update::Down(_)) {
            update = next_main(&mut spawned.updates).await;
        }
        assert_eq!(attached_version(update), "daemon-2");
        let _ = std::fs::remove_file(&socket);
    }

    #[test]
    fn a_long_gap_between_ticks_means_the_mac_slept() {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let mut sleep = Sleep { last: t0 };
        assert!(!sleep.woke_at(t0 + TICK), "an ordinary tick");
        assert!(!sleep.woke_at(t0 + TICK * 2 + Duration::from_millis(300)), "a late tick isn't sleep");
        assert!(sleep.woke_at(t0 + Duration::from_secs(3_600)), "an hour passed between two ticks");
        assert!(!sleep.woke_at(t0 + Duration::from_secs(3_602)), "and then it is quiet again");
        assert!(!sleep.woke_at(t0), "the clock set back isn't sleep");
    }

    // ── over SSH, with a script standing in for `ssh` ──

    /// A folder with a fake `ssh` that prints `body` (a shell script) when run.
    struct FakeSsh {
        dir: PathBuf,
        launcher: Launcher,
    }

    impl FakeSsh {
        fn new(body: &str) -> Self {
            let dir = PathBuf::from(format!("/tmp/eqssh-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let program = dir.join("ssh");
            std::fs::write(&program, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&program, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
            let launcher = Launcher { program, control_dir: dir.clone(), askpass: None, extra: vec![] };
            Self { dir, launcher }
        }

        fn target(&self) -> Target {
            Target {
                endpoint: Endpoint::Ssh(crate::target::SshHost::parse("me@mini").unwrap()),
                prefer: None,
                version: "test".into(),
                ssh: Some(self.launcher.clone()),
            }
        }

        fn file(&self, name: &str, content: &str) -> String {
            let path = self.dir.join(name);
            std::fs::write(&path, content).unwrap();
            path.display().to_string()
        }

        /// A bridge that greets with `attached`, answers the two requests, and
        /// then keeps the line open, copying what the app sends into `received`.
        fn serving(protocol: u32, started: bool) -> Self {
            let attached = Attached { bridge: "0.1".into(), daemon: Some("0.1".into()), protocol, started };
            let fake = Self::new("true");
            let lines = [
                fake.file("attached.json", &Message::notification(rpc::ATTACHED, &attached).to_line()),
                fake.file("squads.json", &Message::response(1, squads()).to_line()),
                fake.file("hello.json", &Message::response(2, hello("remote-1")).to_line()),
            ];
            let received = fake.dir.join("received");
            let script = format!(
                "cat {}\nIFS= read -r _\ncat {}\nIFS= read -r _\ncat {}\nwhile IFS= read -r line; do printf '%s\\n' \"$line\" >> {}; done",
                lines[0],
                lines[1],
                lines[2],
                received.display()
            );
            std::fs::write(fake.dir.join("ssh"), format!("#!/bin/sh\n{script}\n")).unwrap();
            fake
        }

        fn failing(status: i32, stderr: &str) -> Self {
            // Counts its runs, so a test can see whether the link tried again.
            Self::new(&format!("echo run >> \"$(dirname \"$0\")/runs\"\necho '{stderr}' >&2\nexit {status}"))
        }

        fn runs(&self) -> usize {
            std::fs::read_to_string(self.dir.join("runs")).map(|t| t.lines().count()).unwrap_or(0)
        }
    }

    impl Drop for FakeSsh {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[tokio::test]
    async fn attaches_through_the_bridge_like_through_the_socket() {
        let fake = FakeSsh::serving(rpc::PROTOCOL, true);
        let mut spawned = spawn(fake.target(), FAST);

        assert!(matches!(next(&mut spawned.updates).await, Update::Bridge(b) if b.started));
        assert_eq!(next(&mut spawned.updates).await, Update::Squads(squads()));
        assert_eq!(attached_version(next(&mut spawned.updates).await), "remote-1");

        // Commands go down the same line, as JSON-RPC lines.
        spawned.link.send(Command::Interrupt { session: 7 });
        let received = fake.dir.join("received");
        let mut text = String::new();
        for _ in 0..100 {
            text = std::fs::read_to_string(&received).unwrap_or_default();
            if !text.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let message = Message::from_line(text.lines().next().expect("a line reached the far end")).unwrap();
        assert_eq!(message.params_as::<Command>(), Some(Command::Interrupt { session: 7 }));
    }

    #[tokio::test]
    async fn a_dropped_connection_is_retried() {
        // Serves once and hangs up: the second run finds the same script and does it again.
        let fake = FakeSsh::serving(rpc::PROTOCOL, false);
        let script = std::fs::read_to_string(fake.dir.join("ssh")).unwrap();
        std::fs::write(fake.dir.join("ssh"), script.replace("while IFS= read -r line; do", "while false; do")).unwrap();
        let mut spawned = spawn(fake.target(), FAST);
        attached_version(next_main(&mut spawned.updates).await);
        assert!(matches!(next_main(&mut spawned.updates).await, Update::Down(Failure::Closed)));
        attached_version(next_main(&mut spawned.updates).await);
    }

    async fn first_down(fake: &FakeSsh) -> (Failure, Spawned) {
        let mut spawned = spawn(fake.target(), FAST);
        match next(&mut spawned.updates).await {
            Update::Down(failure) => (failure, spawned),
            other => panic!("expected Down, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn each_way_ssh_can_fail_has_its_cause() {
        let cases: [(i32, &str, Failure); 5] = [
            (255, "ssh: connect to host mini port 22: Connection refused", Failure::RemoteLoginOff),
            (127, "sh: 1: exec: equipe: not found", Failure::NoEquipe),
            (69, "equipe attach: the daemon is not running", Failure::DaemonUnavailable("the daemon is not running".into())),
            (255, "Host key verification failed.", Failure::HostKeyRefused),
            (255, "@@@ REMOTE HOST IDENTIFICATION HAS CHANGED! @@@", Failure::HostKeyChanged),
        ];
        for (status, stderr, expected) in cases {
            let fake = FakeSsh::failing(status, stderr);
            assert_eq!(first_down(&fake).await.0, expected, "{stderr}");
        }
        let fake = FakeSsh::failing(255, "me@mini: Permission denied (publickey).");
        assert!(matches!(first_down(&fake).await.0, Failure::AuthFailed(_)));
    }

    #[tokio::test]
    async fn a_version_gap_is_named_before_anything_else_is_asked() {
        let newer = FakeSsh::serving(rpc::PROTOCOL + 1, false);
        assert_eq!(first_down(&newer).await.0, Failure::AppTooOld { daemon: rpc::PROTOCOL + 1 });
        let older = FakeSsh::serving(0, false);
        assert_eq!(first_down(&older).await.0, Failure::DaemonTooOld { daemon: 0 });
    }

    #[tokio::test]
    async fn what_needs_the_person_waits_for_the_person() {
        let fake = FakeSsh::failing(255, "me@mini: Permission denied (publickey,password).");
        let (failure, mut spawned) = first_down(&fake).await;
        assert!(failure.waits_for_person());
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(fake.runs(), 1, "no new ssh (and no new password box) on a timer");

        spawned.link.retry();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(fake.runs(), 2, "the person's click tries once");
        assert!(spawned.updates.try_recv().is_err(), "the same failure is not news");
    }

    #[tokio::test]
    async fn what_is_only_missing_for_now_is_retried_on_a_timer() {
        let fake = FakeSsh::failing(255, "ssh: connect to host mini port 22: Connection refused");
        let (_, _spawned) = first_down(&fake).await;
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(fake.runs() >= 3, "kept trying: {}", fake.runs());
    }
}
