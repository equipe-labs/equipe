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
        writer
    });

    let mut lines = BufReader::new(reader).lines();
    let mut forwarder: Option<tokio::task::JoinHandle<()>> = None;
    // Set when the connection becomes a browser panel's stream.
    let mut stream: Option<(std::sync::Arc<crate::browser::Hub>, u8)> = None;
    // Pairing a phone: this screen's line to the phone link, opened when it asks.
    let mut pairing: Option<crate::pairing::Bridge> = None;
    let mut lang: Option<crate::i18n::Lang> = None;
    // A screen with a browser panel, counted while it is attached.
    let mut panel_screen: Option<std::sync::Arc<crate::browser::Hub>> = None;
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
                // The screen's language is what equipe says to the person in.
                if let Some(code) = params.lang.clone() {
                    lang = crate::i18n::Lang::parse(&code);
                    core.send(Command::Language { lang: code });
                }
                if params.browser_panel
                    && panel_screen.is_none()
                    && let Some(hub) = core.browser_hub().await
                {
                    hub.screen(true);
                    panel_screen = Some(hub);
                }
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
                    if let Command::Language { lang: code } = &command
                        && let Some(now) = crate::i18n::Lang::parse(code)
                    {
                        lang = Some(now);
                        if let Some(bridge) = &mut pairing {
                            bridge.lang = now;
                        }
                    }
                    if crate::pairing::is_pairing(&command) {
                        pairing.get_or_insert_with(|| crate::pairing::Bridge::new(&home, lang)).send(&command, &out_tx).await;
                    } else {
                        core.send(command);
                    }
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
            (Some(rpc::READ_UPLOAD), Some(id)) => {
                // Only a screen that said hello: what it may see was sent on this squad's behalf.
                if forwarder.is_none() {
                    let _ = out_tx.send(Message::failure(id, -32002, "say hello first"));
                    continue;
                }
                let Some(ask) = message.params_as::<rpc::ReadUpload>() else {
                    let _ = out_tx.send(Message::failure(id, -32602, "read_upload needs { path }"));
                    continue;
                };
                let home = home.clone();
                let out = out_tx.clone();
                tokio::spawn(async move {
                    let read = tokio::task::spawn_blocking(move || read_upload(&home, &ask)).await;
                    let _ = out.send(match read {
                        Ok(Ok(read)) => Message::response(id, read),
                        Ok(Err(why)) => Message::failure(id, -32000, why),
                        Err(_) => Message::failure(id, -32000, "the file couldn't be read"),
                    });
                });
            }
            (Some(rpc::DESKTOP), Some(id)) => {
                let Some(call) = message.params_as::<ToolCall>() else {
                    let _ = out_tx.send(Message::failure(id, -32602, "desktop needs { squad, member, name, arguments }"));
                    continue;
                };
                let core = core.clone();
                let out = out_tx.clone();
                tokio::spawn(async move {
                    let result = core.desktop(call).await;
                    let _ = out.send(Message::response(id, result));
                });
            }
            (Some(rpc::BUSY), Some(id)) => {
                let _ = out_tx.send(Message::response(id, core.busy().await));
            }
            (Some(rpc::BROWSER_WINDOW), Some(id)) => {
                let url: String = message.params_as().unwrap_or_default();
                let _ = out_tx.send(Message::response(id, core.person_window(url).await));
            }
            (Some(rpc::BROWSER_STREAM), Some(id)) => {
                let hello = message.params_as::<equipe_protocol::stream::StreamHello>().unwrap_or_default();
                let Some(hub) = core.browser_hub().await else {
                    break;
                };
                let _ = out_tx.send(Message::response(id, true));
                stream = Some((hub, hello.quality.unwrap_or(70).clamp(10, 95)));
                break;
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
    if let Some(hub) = panel_screen {
        hub.screen(false);
    }
    // Its line to the phone link closes, and any code it had open with it.
    drop(pairing);
    drop(out_tx);
    let writer = writer_task.await;
    if let (Some((hub, quality)), Ok(writer)) = (stream, writer) {
        serve_stream(lines.into_inner(), writer, hub, quality).await;
    }
}

/// A browser panel's stream (`equipe_protocol::stream`): frames of the page it
/// watches, as fast as it draws them and no faster than it asks; its input,
/// straight to the page.
async fn serve_stream<R>(mut reader: R, mut writer: OwnedWriteHalf, hub: std::sync::Arc<crate::browser::Hub>, quality: u8)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    use equipe_protocol::stream::{self, Pacer, StreamIn};
    use tokio::io::AsyncReadExt as _;

    hub.panel(true);
    let watcher = hub.watcher();
    let mut frames = hub.frames();
    let (in_tx, mut incoming) = mpsc::unbounded_channel::<StreamIn>();
    let read = tokio::spawn(async move {
        let mut decoder = stream::Decoder::default();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = match reader.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(n) => n,
            };
            decoder.push(&buf[..n]);
            loop {
                match decoder.message() {
                    Ok(Some((stream::JSON, payload))) => {
                        if let Ok(message) = serde_json::from_slice::<StreamIn>(&payload)
                            && in_tx.send(message).is_err()
                        {
                            return;
                        }
                    }
                    Ok(Some(_)) => {}
                    Ok(None) => break,
                    Err(_) => return,
                }
            }
        }
    });
    let mut pacer = Pacer::new(30);
    let mut watching: Option<u64> = None;
    let mut wake: Option<tokio::time::Instant> = None;
    loop {
        let nap = async {
            match wake {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            message = incoming.recv() => match message {
                None => break,
                Some(StreamIn::Watch { page, fps, max_w, max_h, fit }) => {
                    watching = Some(page);
                    pacer.watch(fps);
                    // The box it is shown in: the page is laid out at it — the largest
                    // any panel shows it in — whoever drives it.
                    let fit = fit.filter(|f| f.w >= 64 && f.h >= 64).map(|fit| {
                        let dpr = if fit.dpr.is_finite() { fit.dpr.clamp(1., 4.) } else { 2. };
                        (page, crate::browser::Fit { w: fit.w.min(4096), h: fit.h.min(4096), dpr })
                    });
                    hub.fit(watcher, fit);
                    hub.watch(watcher, crate::browser::Want { page, fps: fps.clamp(1, 60), max_w: max_w.clamp(64, 4096), max_h: max_h.clamp(64, 4096), quality });
                }
                Some(StreamIn::Unwatch) => {
                    watching = None;
                    hub.fit(watcher, None);
                    hub.unwatch(watcher);
                }
                Some(StreamIn::Credit { n }) => pacer.credit(n),
                Some(StreamIn::Input { page, ev }) => {
                    hub.op(&serde_json::json!({ "op": "input", "page": page, "ev": ev }));
                }
            },
            changed = frames.changed() => if changed.is_err() { break },
            () = nap => {}
        }
        wake = None;
        let Some(page) = watching else { continue };
        let Some(frame) = hub.latest(page) else { continue };
        match pacer.take(page, frame.header.seq, std::time::Instant::now()) {
            Ok(true) => {
                let message = stream::encode(stream::FRAME, &stream::frame_payload(&frame.header, &frame.jpeg));
                if writer.write_all(&message).await.is_err() {
                    break;
                }
            }
            Ok(false) => {}
            Err(wait) => wake = Some(tokio::time::Instant::now() + wait),
        }
    }
    hub.unwatch(watcher);
    hub.panel(false);
    read.abort();
}

fn squads(home: &Path) -> rpc::Squads {
    let squads = crate::squads::list(home)
        .into_iter()
        .map(|s| rpc::SquadEntry { home: crate::squad_home::path(home, &s.name), name: s.name, projects: s.projects })
        .collect();
    rpc::Squads { squads, last: crate::squads::default_squad(home) }
}

/// An `upload` line carries at most this much (≈ 30 MB of file); bigger
/// files come over their own channel (`equipe receive`).
const UPLOAD_MAX: usize = 40 << 20;
/// What `equipe receive` takes at most.
pub const RECEIVE_MAX: u64 = 2 << 30;
/// Uploads older than this are swept when another arrives.
const UPLOAD_KEEP: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 3600);
const IMAGE_KINDS: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];

/// Where uploads are kept on this machine.
pub fn uploads_dir(home: &Path) -> PathBuf {
    home.join("uploads")
}

/// The stem and extension an upload is kept under: the extension lowercased,
/// the stem reduced to letters, digits, `-` and `_` (no folders, no dots) —
/// or why `kind` doesn't take that name.
pub fn upload_name(name: &str, kind: Option<rpc::UploadKind>) -> Result<(String, String), String> {
    use rpc::UploadKind as K;
    let path = Path::new(name.rsplit(['/', '\\']).next().unwrap_or(name));
    let ext: String = path.extension().and_then(|e| e.to_str()).unwrap_or("").chars().filter(char::is_ascii_alphanumeric).take(10).collect::<String>().to_ascii_lowercase();
    match kind {
        None | Some(K::Image) if !IMAGE_KINDS.contains(&ext.as_str()) => {
            return Err(format!("only images can be uploaded ({})", IMAGE_KINDS.join(", ")));
        }
        Some(K::Pdf) if ext != "pdf" => return Err("a PDF has to end in .pdf".into()),
        _ => {}
    }
    let mut stem = String::new();
    for c in path.file_stem().and_then(|s| s.to_str()).unwrap_or("").chars() {
        let c = if c.is_alphanumeric() || c == '_' { c } else { '-' };
        if !(c == '-' && (stem.is_empty() || stem.ends_with('-'))) {
            stem.push(c);
        }
        if stem.chars().count() >= 48 {
            break;
        }
    }
    let stem = stem.trim_end_matches('-').to_owned();
    Ok((if stem.is_empty() { "upload".into() } else { stem }, ext))
}

/// A new file under `<home>/uploads` (0600 in a 0700 folder) for `name`,
/// named by equipe — `<stamp>-<stem>.<ext>`, never a folder of the screen's
/// choosing — and old uploads swept while there.
pub fn create_upload(home: &Path, name: &str, kind: Option<rpc::UploadKind>) -> Result<(std::fs::File, PathBuf), String> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let (stem, ext) = upload_name(name, kind)?;
    let dir = uploads_dir(home);
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
    let dot = if ext.is_empty() { String::new() } else { format!(".{ext}") };
    for n in 0.. {
        let again = if n == 0 { String::new() } else { format!("-{n}") };
        let path = dir.join(format!("{stamp}-{stem}{again}{dot}"));
        match std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path) {
            Ok(file) => return Ok((file, path)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    unreachable!("some name is free")
}

/// Keep an `upload` line's file.
fn save_upload(home: &Path, upload: &rpc::Upload) -> Result<PathBuf, String> {
    use base64::Engine as _;
    use std::io::Write as _;
    upload_name(&upload.name, upload.kind)?;
    if upload.data.len() > UPLOAD_MAX / 3 * 4 + 4 {
        return Err("the file is too big to send this way".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(upload.data.trim()).map_err(|_| "the file isn't base64".to_owned())?;
    let (mut file, path) = create_upload(home, &upload.name, upload.kind)?;
    if let Err(e) = file.write_all(&bytes) {
        let _ = std::fs::remove_file(&path);
        return Err(e.to_string());
    }
    Ok(path)
}

/// Keep exactly `size` bytes read from `from` (`equipe receive`, a big file
/// streamed over SSH). Anything short of `size` keeps nothing.
pub fn receive_upload(home: &Path, name: &str, kind: Option<rpc::UploadKind>, size: u64, from: impl io::Read) -> Result<PathBuf, String> {
    if size > RECEIVE_MAX {
        return Err(format!("the file is bigger than {} GB", RECEIVE_MAX >> 30));
    }
    let (mut file, path) = create_upload(home, name, kind)?;
    let copied = io::copy(&mut from.take(size), &mut file);
    match copied {
        Ok(n) if n == size => Ok(path),
        Ok(n) => {
            let _ = std::fs::remove_file(&path);
            Err(format!("the file ended after {n} of {size} bytes"))
        }
        Err(e) => {
            let _ = std::fs::remove_file(&path);
            Err(e.to_string())
        }
    }
}

/// The most `read_upload` gives back: a picture is at most 8 MB once a screen
/// made it ready, a video's frames far less.
pub const READ_UPLOAD_MAX: u64 = 12 << 20;

/// `path`, when it is a file of this machine's uploads — and nothing else: no
/// `..`, not a link, not in a folder under it, not reached through a link out
/// of it. What a screen asks back is only what screens put there.
pub fn upload_file(home: &Path, path: &Path) -> Result<PathBuf, String> {
    use std::path::Component;
    let refused = || "not an upload".to_owned();
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        return Err(refused());
    }
    let meta = std::fs::symlink_metadata(path).map_err(|_| "the file is gone".to_owned())?;
    if !meta.file_type().is_file() {
        return Err(refused());
    }
    let dir = std::fs::canonicalize(uploads_dir(home)).map_err(|_| refused())?;
    let real = std::fs::canonicalize(path).map_err(|_| refused())?;
    if real.parent() != Some(dir.as_path()) {
        return Err(refused());
    }
    Ok(real)
}

/// Answer a `read_upload`: the file's size, and its bytes unless only the
/// size was asked; one bigger than [`READ_UPLOAD_MAX`] is refused.
pub fn read_upload(home: &Path, ask: &rpc::ReadUpload) -> Result<rpc::UploadRead, String> {
    use base64::Engine as _;
    use io::Read as _;
    let path = upload_file(home, &ask.path)?;
    let mut file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    if ask.size_only {
        return Ok(rpc::UploadRead { size, data: None });
    }
    let too_big = || format!("the file is bigger than {} MB", READ_UPLOAD_MAX >> 20);
    if size > READ_UPLOAD_MAX {
        return Err(too_big());
    }
    let mut bytes = Vec::new();
    file.by_ref().take(READ_UPLOAD_MAX + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > READ_UPLOAD_MAX {
        return Err(too_big());
    }
    Ok(rpc::UploadRead { size: bytes.len() as u64, data: Some(base64::engine::general_purpose::STANDARD.encode(&bytes)) })
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
        Self::connect_in(socket, version, squad, None).await
    }

    /// [`Client::connect`], saying the screen speaks `lang`: what the daemon
    /// says to the person is in it from now on.
    pub async fn connect_in(socket: &Path, version: &str, squad: &str, lang: Option<crate::i18n::Lang>) -> io::Result<(Self, Hello)> {
        let stream = UnixStream::connect(socket).await?;
        let (reader, mut writer) = stream.into_split();
        let params = HelloParams { version: version.to_owned(), squad: squad.to_owned(), lang: lang.map(|l| l.code().to_owned()), browser_panel: false };
        let hello = Message::request(1, rpc::HELLO, params);
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
    call_on(socket, rpc::TOOL, call).await
}

/// Call a member's desktop tool through the daemon at `socket`.
pub async fn call_desktop(socket: &Path, call: &ToolCall) -> io::Result<ToolResult> {
    call_on(socket, rpc::DESKTOP, call).await
}

async fn call_on(socket: &Path, method: &str, call: &ToolCall) -> io::Result<ToolResult> {
    let stream = UnixStream::connect(socket).await?;
    let (reader, mut writer) = stream.into_split();
    write_line(&mut writer, &Message::request(1, method, call)).await?;
    let mut lines = BufReader::new(reader).lines();
    while let Some(line) = lines.next_line().await? {
        let Ok(message) = Message::from_line(&line) else {
            continue;
        };
        if message.id != Some(1) {
            continue;
        }
        if let Some(error) = message.error {
            return Ok(ToolResult { text: error.message, is_error: true, image: None });
        }
        return message.result_as::<ToolResult>().ok_or_else(|| io::Error::other("the daemon answered something unexpected"));
    }
    Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the daemon closed the connection"))
}

/// Open a window for the person in the members' shared Chrome, at `url`.
/// `Ok(false)` when no daemon answers or members use a Chrome each.
pub async fn person_window(socket: &Path, url: &str) -> io::Result<bool> {
    let Ok(stream) = UnixStream::connect(socket).await else {
        return Ok(false);
    };
    let (reader, mut writer) = stream.into_split();
    write_line(&mut writer, &Message::request(1, rpc::BROWSER_WINDOW, url)).await?;
    let mut lines = BufReader::new(reader).lines();
    while let Some(line) = lines.next_line().await? {
        let Ok(message) = Message::from_line(&line) else {
            continue;
        };
        if message.id == Some(1) {
            return Ok(message.result_as::<bool>().unwrap_or(false));
        }
    }
    Ok(false)
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

#[cfg(test)]
mod upload_tests {
    use super::*;
    use rpc::UploadKind as K;

    fn home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-uploads-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn names_keep_a_tidy_stem_and_the_extension_and_never_a_folder() {
        assert_eq!(upload_name("../../Screen Shot 2026.PNG", None), Ok(("Screen-Shot-2026".into(), "png".into())));
        assert_eq!(upload_name("/etc/기획서 v2.pdf", Some(K::Pdf)), Ok(("기획서-v2".into(), "pdf".into())));
        assert_eq!(upload_name("..hidden.md", Some(K::File)), Ok(("hidden".into(), "md".into())));
        assert_eq!(upload_name("Makefile", Some(K::File)), Ok(("Makefile".into(), String::new())));
        assert_eq!(upload_name("!!!.txt", Some(K::File)), Ok(("upload".into(), "txt".into())));
        assert!(upload_name("notes.sh", None).is_err(), "without a kind, only images");
        assert!(upload_name("notes.sh", Some(K::Image)).is_err());
        assert!(upload_name("x.docx", Some(K::Pdf)).is_err());
        assert!(upload_name("clip.mov", Some(K::Video)).is_ok());
    }

    #[test]
    fn a_line_upload_is_kept_private_under_its_name() {
        use base64::Engine as _;
        use std::os::unix::fs::PermissionsExt as _;
        let home = home("line");
        let data = base64::engine::general_purpose::STANDARD.encode("# 메모\n");
        let path = save_upload(&home, &rpc::Upload { name: "회의 메모.md".into(), data, kind: Some(K::File) }).unwrap();
        assert_eq!(path.parent(), Some(uploads_dir(&home).as_path()));
        assert!(path.file_name().unwrap().to_str().unwrap().ends_with("-회의-메모.md"), "{path:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# 메모\n");
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        // The same name twice: two files.
        let again = save_upload(&home, &rpc::Upload { name: "회의 메모.md".into(), data: "AA==".into(), kind: Some(K::File) }).unwrap();
        assert_ne!(again, path);
        std::fs::remove_dir_all(home).unwrap();
    }

    fn read(home: &Path, path: &Path) -> Result<Vec<u8>, String> {
        use base64::Engine as _;
        let read = read_upload(home, &rpc::ReadUpload { path: path.to_owned(), size_only: false })?;
        Ok(base64::engine::general_purpose::STANDARD.decode(read.data.unwrap()).unwrap())
    }

    #[test]
    fn only_a_file_in_the_uploads_folder_is_read_back() {
        use std::os::unix::fs::symlink;
        let home = home("read");
        let (_, shot) = create_upload(&home, "shot.png", None).unwrap();
        std::fs::write(&shot, b"png bytes").unwrap();
        assert_eq!(read(&home, &shot).unwrap(), b"png bytes");
        let sized = read_upload(&home, &rpc::ReadUpload { path: shot.clone(), size_only: true }).unwrap();
        assert_eq!((sized.size, sized.data), (9, None));

        // Beside the uploads, outside them, and a way out through `..`.
        let secret = home.join("credentials.json");
        std::fs::write(&secret, b"{}").unwrap();
        let uploads = uploads_dir(&home);
        assert!(read(&home, &secret).is_err());
        assert!(read(&home, &uploads.join("../credentials.json")).is_err());
        assert!(read(&home, Path::new("uploads/x.png")).is_err(), "only absolute paths");
        assert!(read(&home, Path::new("/etc/passwd")).is_err());
        // A link in the uploads to a file outside, or to the folder above.
        symlink(&secret, uploads.join("link.png")).unwrap();
        assert!(read(&home, &uploads.join("link.png")).is_err());
        symlink(&home, uploads.join("up")).unwrap();
        assert!(read(&home, &uploads.join("up/credentials.json")).is_err());
        // A folder under the uploads, the folder itself, a file that is gone.
        std::fs::create_dir(uploads.join("sub")).unwrap();
        std::fs::write(uploads.join("sub/a.png"), b"x").unwrap();
        assert!(read(&home, &uploads.join("sub/a.png")).is_err());
        assert!(read(&home, &uploads).is_err());
        assert!(read(&home, &uploads.join("gone.png")).is_err());
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn an_uploads_folder_reached_through_a_link_still_reads_its_own_files() {
        let real = home("linked-real");
        let (_, shot) = create_upload(&real, "a.png", None).unwrap();
        std::fs::write(&shot, b"ok").unwrap();
        let linked = home("linked");
        std::os::unix::fs::symlink(&real, &linked).unwrap();
        let through = uploads_dir(&linked).join(shot.file_name().unwrap());
        assert_eq!(read(&linked, &through).unwrap(), b"ok");
        std::fs::remove_file(linked).unwrap();
        std::fs::remove_dir_all(real).unwrap();
    }

    #[test]
    fn a_file_too_big_to_read_back_is_refused_but_its_size_is_told() {
        let home = home("big");
        let (file, big) = create_upload(&home, "big.png", None).unwrap();
        file.set_len(READ_UPLOAD_MAX + 1).unwrap();
        assert!(read(&home, &big).unwrap_err().contains("MB"));
        assert_eq!(read_upload(&home, &rpc::ReadUpload { path: big, size_only: true }).unwrap().size, READ_UPLOAD_MAX + 1);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn a_streamed_upload_keeps_exactly_its_size_or_nothing() {
        let home = home("stream");
        let path = receive_upload(&home, "clip.mov", Some(K::Video), 5, &b"12345678"[..]).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"12345");
        assert!(receive_upload(&home, "short.mov", Some(K::Video), 10, &b"123"[..]).is_err());
        let left: Vec<_> = std::fs::read_dir(uploads_dir(&home)).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(left.len(), 1, "the short one leaves nothing behind: {left:?}");
        std::fs::remove_dir_all(home).unwrap();
    }
}
