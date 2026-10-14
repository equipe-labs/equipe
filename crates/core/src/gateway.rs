//! The daemon's door: the runtime served over a Unix socket.
//!
//! One daemon per user, at `~/.equipe/equipe.sock`. The socket is a file only
//! its owner can open, so no network port is involved and nothing else on the
//! machine can drive the squad. Messages are the protocol's Commands and Events
//! in JSON-RPC 2.0, one per line (see `equipe_protocol::rpc`).
//!
//! Screens come and go; the runtime — and the agents it runs, which are the
//! daemon's children, not any terminal's — keeps going.

use std::io;
use std::path::{Path, PathBuf};

use equipe_protocol::rpc::{self, Hello, HelloParams, Message, ToolCall, ToolResult};
use equipe_protocol::{Command, Event};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;

use crate::runtime::{self, Handle};

/// Where the daemon listens.
pub fn socket_path(home: &Path) -> PathBuf {
    home.join("equipe.sock")
}

/// Serve the runtime on `socket` until a client asks it to shut down.
/// Fails with `AddrInUse` when a daemon is already answering there.
pub async fn serve(config: runtime::Config, socket: &Path, version: &str) -> io::Result<()> {
    let listener = bind(socket).await?;
    let home = config.home.clone();
    let (core, own_events) = runtime::start(config);
    // Screens subscribe for themselves; an unread receiver would only pile up events.
    drop(own_events);
    let (stop_tx, mut stop_rx) = mpsc::unbounded_channel::<oneshot::Sender<()>>();

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                tokio::spawn(serve_client(stream, core.clone(), home.clone(), version.to_owned(), stop_tx.clone()));
            }
            Some(ack) = stop_rx.recv() => {
                core.shutdown().await;
                let _ = std::fs::remove_file(socket);
                let _ = ack.send(());
                return Ok(());
            }
        }
    }
}

/// Bind the socket, replacing a stale file left by a daemon that died.
async fn bind(socket: &Path) -> io::Result<UnixListener> {
    if socket.exists() {
        if UnixStream::connect(socket).await.is_ok() {
            return Err(io::Error::new(io::ErrorKind::AddrInUse, "a daemon is already running"));
        }
        std::fs::remove_file(socket)?;
    }
    if let Some(dir) = socket.parent() {
        std::fs::create_dir_all(dir)?;
        restrict(dir, 0o700)?;
    }
    let listener = UnixListener::bind(socket)?;
    restrict(socket, 0o600)?;
    Ok(listener)
}

fn restrict(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

async fn write_line(writer: &mut OwnedWriteHalf, message: &Message) -> io::Result<()> {
    writer.write_all(message.to_line().as_bytes()).await
}

/// One screen's connection: answer requests, forward its events.
async fn serve_client(stream: UnixStream, core: Handle, home: PathBuf, version: String, stop: UnboundedSender<oneshot::Sender<()>>) {
    let (reader, writer) = stream.into_split();
    // Replies and events share the write half; a task owns it.
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Message>();
    let writer_task = tokio::spawn(async move {
        let mut writer = writer;
        while let Some(message) = out_rx.recv().await {
            if write_line(&mut writer, &message).await.is_err() {
                break;
            }
        }
    });

    let mut lines = BufReader::new(reader).lines();
    let mut forwarder: Option<tokio::task::JoinHandle<()>> = None;
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(message) = Message::from_line(&line) else {
            continue;
        };
        match (message.method.as_deref(), message.id) {
            (Some(rpc::HELLO), Some(id)) => {
                let Some(params) = message.params_as::<HelloParams>() else {
                    let _ = out_tx.send(Message::failure(id, -32602, "hello needs { version, squad }"));
                    continue;
                };
                let Some((snapshot, mut events)) = core.subscribe(&params.squad).await else {
                    break;
                };
                let _ = out_tx.send(Message::response(id, Hello { version: version.clone(), snapshot }));
                let out = out_tx.clone();
                forwarder.replace(tokio::spawn(async move {
                    while let Some(event) = events.recv().await {
                        if out.send(Message::event(&event)).is_err() {
                            break;
                        }
                    }
                }));
            }
            (Some(rpc::COMMAND), _) => {
                if let Some(command) = message.params_as::<Command>() {
                    core.send(command);
                }
            }
            (Some(rpc::TOOL), Some(id)) => {
                let Some(call) = message.params_as::<ToolCall>() else {
                    let _ = out_tx.send(Message::failure(id, -32602, "tool needs { squad, member, name, arguments }"));
                    continue;
                };
                // Answered on its own task: a tool call may wait while the runtime
                // is busy, and this connection must keep reading meanwhile.
                let core = core.clone();
                let out = out_tx.clone();
                tokio::spawn(async move {
                    let result = core.tool(call).await;
                    let _ = out.send(Message::response(id, result));
                });
            }
            (Some(rpc::VERSION), Some(id)) => {
                let _ = out_tx.send(Message::response(id, rpc::Version { version: version.clone(), protocol: rpc::PROTOCOL }));
            }
            (Some(rpc::SQUADS), Some(id)) => {
                let home = home.clone();
                let squads = tokio::task::spawn_blocking(move || squads(&home)).await.unwrap_or_default();
                let _ = out_tx.send(Message::response(id, squads));
            }
            (Some(rpc::UPLOAD), Some(id)) => {
                let Some(upload) = message.params_as::<rpc::Upload>() else {
                    let _ = out_tx.send(Message::failure(id, -32602, "upload needs { name, data }"));
                    continue;
                };
                let home = home.clone();
                let out = out_tx.clone();
                tokio::spawn(async move {
                    let saved = tokio::task::spawn_blocking(move || save_upload(&home, &upload)).await;
                    let _ = out.send(match saved {
                        Ok(Ok(path)) => Message::response(id, rpc::Uploaded { path }),
                        Ok(Err(why)) => Message::failure(id, -32000, why),
                        Err(_) => Message::failure(id, -32000, "the upload failed"),
                    });
                });
            }
            (Some(rpc::BUSY), Some(id)) => {
                let _ = out_tx.send(Message::response(id, core.busy().await));
            }
            (Some(rpc::SHUTDOWN), Some(id)) => {
                let (ack_tx, ack_rx) = oneshot::channel();
                let _ = stop.send(ack_tx);
                let _ = ack_rx.await;
                let _ = out_tx.send(Message::response(id, "stopped"));
                break;
            }
            (Some(_), Some(id)) => {
                let _ = out_tx.send(Message::failure(id, -32601, "unknown method"));
            }
            _ => {}
        }
    }
    // The screen left. Its agents keep working; only its view goes away.
    if let Some(forwarder) = forwarder {
        forwarder.abort();
    }
    drop(out_tx);
    let _ = writer_task.await;
}

fn squads(home: &Path) -> rpc::Squads {
    let squads = crate::squads::list(home)
        .into_iter()
        .map(|s| rpc::SquadEntry { home: crate::squad_home::path(home, &s.name), name: s.name, projects: s.projects })
        .collect();
    rpc::Squads { squads, last: crate::squads::default_squad(home) }
}

/// Uploads bigger than this are refused (≈ 30 MB of picture).
const UPLOAD_MAX: usize = 40 << 20;
/// Uploads older than this are swept when another arrives.
const UPLOAD_KEEP: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 3600);
const UPLOAD_KINDS: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];

/// Keep an uploaded image under `<home>/uploads` (0600 in a 0700 folder), named
/// by equipe — never by the screen — so it can't land anywhere else.
fn save_upload(home: &Path, upload: &rpc::Upload) -> Result<PathBuf, String> {
    use base64::Engine as _;
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    let kind = Path::new(&upload.name).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    if !UPLOAD_KINDS.contains(&kind.as_str()) {
        return Err(format!("only images can be uploaded ({})", UPLOAD_KINDS.join(", ")));
    }
    if upload.data.len() > UPLOAD_MAX / 3 * 4 + 4 {
        return Err("the image is too big".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(upload.data.trim()).map_err(|_| "the image isn't base64".to_owned())?;
    let dir = home.join("uploads");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    restrict(&dir, 0o700).map_err(|e| e.to_string())?;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let old = entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > UPLOAD_KEEP);
            if old {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
    for n in 0.. {
        let path = dir.join(format!("upload-{stamp}-{n}.{kind}"));
        match std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path) {
            Ok(mut file) => {
                file.write_all(&bytes).map_err(|e| e.to_string())?;
                return Ok(path);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    unreachable!("some name is free")
}

/// Where a split [`Client`] sends its commands.
#[derive(Clone)]
pub struct Commands(UnboundedSender<Message>);

impl Commands {
    pub fn send(&self, command: Command) {
        let _ = self.0.send(Message::command(&command));
    }
}

/// A screen's side of the socket.
pub struct Client {
    commands: UnboundedSender<Message>,
    events: UnboundedReceiver<Event>,
}

impl Client {
    /// Attach to the daemon and follow `squad`. Returns the daemon's greeting,
    /// with the squad's snapshot.
    pub async fn connect(socket: &Path, version: &str, squad: &str) -> io::Result<(Self, Hello)> {
        let stream = UnixStream::connect(socket).await?;
        let (reader, mut writer) = stream.into_split();
        let hello = Message::request(1, rpc::HELLO, HelloParams { version: version.to_owned(), squad: squad.to_owned() });
        write_line(&mut writer, &hello).await?;

        let mut lines = BufReader::new(reader).lines();
        let greeting = loop {
            let line = lines
                .next_line()
                .await?
                .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "the daemon closed the connection"))?;
            let message = Message::from_line(&line).map_err(io::Error::other)?;
            if message.id == Some(1) {
                break message
                    .result_as::<Hello>()
                    .ok_or_else(|| io::Error::other("the daemon refused the hello"))?;
            }
        };

        let (evt_tx, evt_rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(message) = Message::from_line(&line)
                    && message.method.as_deref() == Some(rpc::EVENT)
                    && let Some(event) = message.params_as::<Event>()
                    && evt_tx.send(event).is_err()
                {
                    break;
                }
            }
        });
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<Message>();
        tokio::spawn(async move {
            while let Some(message) = cmd_rx.recv().await {
                if write_line(&mut writer, &message).await.is_err() {
                    break;
                }
            }
        });
        Ok((Self { commands: cmd_tx, events: evt_rx }, greeting))
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(Message::command(&command));
    }

    /// Its two directions apart: a sender to keep (and clone), and the events.
    pub fn split(self) -> (Commands, UnboundedReceiver<Event>) {
        (Commands(self.commands), self.events)
    }

    /// The next event, or `None` once the daemon is gone.
    pub async fn next_event(&mut self) -> Option<Event> {
        self.events.recv().await
    }

    /// An event already waiting, without waiting for one.
    pub fn try_event(&mut self) -> Option<Event> {
        self.events.try_recv().ok()
    }
}

/// Ask the daemon at `socket` to stop. Its open sessions stay journaled and
/// resume when it starts again. `Ok(false)` when no daemon was running.
pub async fn shutdown(socket: &Path) -> io::Result<bool> {
    let Ok(stream) = UnixStream::connect(socket).await else {
        return Ok(false);
    };
    let (reader, mut writer) = stream.into_split();
    write_line(&mut writer, &Message::request(1, rpc::SHUTDOWN, ())).await?;
    let mut lines = BufReader::new(reader).lines();
    while let Some(line) = lines.next_line().await? {
        if Message::from_line(&line).map(|m| m.id == Some(1)).unwrap_or(false) {
            return Ok(true);
        }
    }
    Ok(true)
}

/// What the daemon at `socket` is in the middle of; `None` when none answers.
pub async fn busy(socket: &Path) -> io::Result<Option<rpc::Busy>> {
    let Ok(stream) = UnixStream::connect(socket).await else {
        return Ok(None);
    };
    let (reader, mut writer) = stream.into_split();
    write_line(&mut writer, &Message::request(1, rpc::BUSY, ())).await?;
    let mut lines = BufReader::new(reader).lines();
    while let Some(line) = lines.next_line().await? {
        let Ok(message) = Message::from_line(&line) else {
            continue;
        };
        if message.id == Some(1) {
            return Ok(message.result_as::<rpc::Busy>());
        }
    }
    Ok(None)
}

/// Call a member's board tool on the daemon at `socket` — what `equipe mcp`
/// does for each call its engine makes.
pub async fn call_tool(socket: &Path, call: &ToolCall) -> io::Result<ToolResult> {
    let stream = UnixStream::connect(socket).await?;
    let (reader, mut writer) = stream.into_split();
    write_line(&mut writer, &Message::request(1, rpc::TOOL, call)).await?;
    let mut lines = BufReader::new(reader).lines();
    while let Some(line) = lines.next_line().await? {
        let Ok(message) = Message::from_line(&line) else {
            continue;
        };
        if message.id != Some(1) {
            continue;
        }
        if let Some(error) = message.error {
            return Ok(ToolResult { text: error.message, is_error: true });
        }
        return message.result_as::<ToolResult>().ok_or_else(|| io::Error::other("the daemon answered something unexpected"));
    }
    Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the daemon closed the connection"))
}

/// Which daemon answers at `socket`: `Ok(None)` when it is older than the
/// `version` method.
pub async fn version(socket: &Path) -> io::Result<Option<rpc::Version>> {
    let stream = UnixStream::connect(socket).await?;
    let (reader, mut writer) = stream.into_split();
    write_line(&mut writer, &Message::request(1, rpc::VERSION, ())).await?;
    let mut lines = BufReader::new(reader).lines();
    while let Some(line) = lines.next_line().await? {
        let Ok(message) = Message::from_line(&line) else {
            continue;
        };
        if message.id == Some(1) {
            return Ok(message.result_as::<rpc::Version>());
        }
    }
    Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the daemon closed the connection"))
}

/// Whether a daemon answers at `socket`.
pub async fn is_running(socket: &Path) -> bool {
    UnixStream::connect(socket).await.is_ok()
}
