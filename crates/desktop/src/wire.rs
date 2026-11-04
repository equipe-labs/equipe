//! The daemon's protocol over any pair of byte streams: the socket's two
//! halves on this Mac, or an `ssh … equipe attach --stdio` child's stdout and
//! stdin. One code path for both — what differs is only whether the bridge's
//! `attached` line comes first.
//!
//! Handshake: `[attached]` → `squads` request → choose the squad → `hello`.
//! After it, events come out of one channel and commands go into another.

use equipe_core::t;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use equipe_core::protocol::rpc::{self, Attached, Hello, HelloParams, Message, Squads};
use equipe_core::protocol::{Command, Event};
use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncWrite, AsyncWriteExt as _, BufReader};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::failure::Failure;

const SQUADS_ID: u64 = 1;
const HELLO_ID: u64 = 2;
/// Requests after the handshake count up from here.
const FIRST_ASKED_ID: u64 = 100;

/// How the handshake goes.
#[derive(Debug, Clone)]
pub struct Opts {
    /// What the app says it is in `hello`.
    pub version: String,
    /// The squad to follow if it exists; else the daemon's `last`, else the first.
    pub prefer: Option<String>,
    /// Through `equipe attach --stdio`: the first line is the bridge's.
    pub bridged: bool,
    /// How long until the first line (or the first answer): SSH may be waiting
    /// for the person to type a password.
    pub first: Duration,
    /// How long each later step may take.
    pub step: Duration,
}

/// What the handshake learned.
#[derive(Debug, Clone)]
pub struct Greeting {
    pub bridge: Option<Attached>,
    pub squads: Squads,
    pub hello: Hello,
}

/// Answers still awaited, by request id: the waiting thread's end of each,
/// given the answer's result (or its error's message).
type Pending = Arc<Mutex<HashMap<u64, std::sync::mpsc::Sender<Result<serde_json::Value, String>>>>>;

/// Where a split connection's commands go.
#[derive(Clone)]
pub struct Commands {
    out: UnboundedSender<Message>,
    pending: Pending,
    next: Arc<AtomicU64>,
}

impl Commands {
    /// In the real view (`readonly.rs`) only a command that reads leaves.
    pub fn send(&self, command: Command) {
        if crate::readonly::lets_command(&command) {
            let _ = self.out.send(Message::command(&command));
        }
    }

    /// Ask the daemon to keep a file (`upload`; `kind: None`, an image, as
    /// before attachments); the answer, its path on the daemon's machine,
    /// comes on the returned channel — or it is dropped unanswered when the
    /// connection ends.
    pub fn upload(
        &self,
        name: &str,
        bytes: &[u8],
        kind: Option<rpc::UploadKind>,
    ) -> std::sync::mpsc::Receiver<Result<serde_json::Value, String>> {
        use base64::Engine as _;
        let params = rpc::Upload {
            name: name.to_owned(),
            data: base64::engine::general_purpose::STANDARD.encode(bytes),
            kind,
        };
        self.ask(rpc::UPLOAD, params)
    }

    /// Ask for a file an attachment tag points at (`read_upload`), back from
    /// the daemon's uploads; answered as [`Commands::upload`] is.
    pub fn read_upload(&self, path: &str, size_only: bool) -> std::sync::mpsc::Receiver<Result<serde_json::Value, String>> {
        self.ask(rpc::READ_UPLOAD, rpc::ReadUpload { path: path.into(), size_only })
    }

    /// Ask for a member's picture (`read_image`) — no bigger than `max`
    /// pixels on its longer side; answered as [`Commands::upload`] is.
    pub fn read_image(&self, squad: &str, path: &str, max: Option<u32>) -> std::sync::mpsc::Receiver<Result<serde_json::Value, String>> {
        self.ask(rpc::READ_IMAGE, rpc::ReadImage { squad: squad.to_owned(), path: path.to_owned(), max })
    }

    /// Ask for the pictures in a folder a reply names (`list_images`);
    /// answered as [`Commands::upload`] is.
    pub fn list_images(&self, squad: &str, folder: &str) -> std::sync::mpsc::Receiver<Result<serde_json::Value, String>> {
        self.ask(rpc::LIST_IMAGES, rpc::ListImages { squad: squad.to_owned(), folder: folder.to_owned(), max: None })
    }

    /// Whether anonymous usage events are sent (`telemetry`); answered as
    /// [`Commands::upload`] is.
    pub fn telemetry(&self) -> std::sync::mpsc::Receiver<Result<serde_json::Value, String>> {
        self.ask(rpc::TELEMETRY, serde_json::json!({}))
    }

    /// The person's yes or no to those events (`telemetry_set`).
    pub fn telemetry_set(&self, on: bool) -> std::sync::mpsc::Receiver<Result<serde_json::Value, String>> {
        self.ask(rpc::TELEMETRY_SET, equipe_core::protocol::telemetry::SetTelemetry { on })
    }

    /// Tell the daemon something that wants no answer (`screen_look`). In
    /// the real view only what reads leaves — and nothing told does.
    pub fn tell(&self, method: &str, params: impl serde::Serialize) {
        if crate::readonly::lets_method(method) {
            let _ = self.out.send(Message::notification(method, params));
        }
    }

    /// Any request of the daemon's JSON-RPC (`metrics.*`); answered as [`Commands::upload`] is.
    /// In the real view only one that reads leaves; the rest is answered no here.
    pub fn ask(&self, method: &str, params: impl serde::Serialize) -> std::sync::mpsc::Receiver<Result<serde_json::Value, String>> {
        let (tx, rx) = std::sync::mpsc::channel();
        if !crate::readonly::lets_method(method) {
            let _ = tx.send(Err(t!("readonly.kept_back").to_owned()));
            return rx;
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.pending.lock().unwrap().insert(id, tx);
        if self.out.send(Message::request(id, method, params)).is_err() {
            self.pending.lock().unwrap().remove(&id);
        }
        rx
    }
}

pub struct Wire {
    pub commands: Commands,
    pub events: UnboundedReceiver<Event>,
}

/// Why a handshake stopped.
#[derive(Debug)]
pub enum Stop {
    /// The stream ended: for SSH, the process's exit says why.
    Eof,
    Failed(Failure),
}

impl From<Failure> for Stop {
    fn from(f: Failure) -> Self {
        Self::Failed(f)
    }
}

/// The squad to follow among `squads`.
pub fn choose_squad(squads: &Squads, prefer: Option<&str>) -> Option<String> {
    let has = |name: &str| squads.squads.iter().any(|s| s.name == name);
    prefer
        .filter(|p| has(p))
        .map(str::to_owned)
        .or_else(|| squads.last.clone().filter(|l| has(l)))
        .or_else(|| squads.squads.first().map(|s| s.name.clone()))
}

/// Compare the bridge's wire version with ours.
pub fn check_protocol(attached: &Attached) -> Result<(), Failure> {
    use std::cmp::Ordering::*;
    match attached.protocol.cmp(&rpc::PROTOCOL) {
        Equal => Ok(()),
        Less => Err(Failure::DaemonTooOld { daemon: attached.protocol }),
        Greater => Err(Failure::AppTooOld { daemon: attached.protocol }),
    }
}

async fn next_message<R: tokio::io::AsyncBufRead + Unpin>(lines: &mut tokio::io::Lines<R>, wait: Duration) -> Result<Message, Stop> {
    loop {
        let line = match tokio::time::timeout(wait, lines.next_line()).await {
            Err(_) => return Err(Failure::NoAnswer.into()),
            Ok(Err(_)) | Ok(Ok(None)) => return Err(Stop::Eof),
            Ok(Ok(Some(line))) => line,
        };
        if let Ok(message) = Message::from_line(&line) {
            return Ok(message);
        }
    }
}

async fn send<W: AsyncWrite + Unpin>(writer: &mut W, message: &Message) -> Result<(), Stop> {
    writer.write_all(message.to_line().as_bytes()).await.map_err(|_| Stop::Eof)?;
    writer.flush().await.map_err(|_| Stop::Eof)
}

/// Read until the response to `id`; a failure response becomes `Other`.
async fn response<R: tokio::io::AsyncBufRead + Unpin>(lines: &mut tokio::io::Lines<R>, id: u64, wait: Duration, what: &str) -> Result<Message, Stop> {
    loop {
        let message = next_message(lines, wait).await?;
        if message.id == Some(id) && message.method.is_none() {
            if let Some(error) = &message.error {
                return Err(Failure::Other(format!("{what}: {}", error.message)).into());
            }
            return Ok(message);
        }
    }
}

/// Run the handshake and start following the daemon.
pub async fn connect<R, W>(reader: R, mut writer: W, opts: &Opts) -> Result<(Greeting, Wire), Stop>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let mut lines = BufReader::new(reader).lines();
    let mut wait = opts.first;

    let mut bridge = None;
    if opts.bridged {
        loop {
            let message = next_message(&mut lines, wait).await?;
            if message.method.as_deref() == Some(rpc::ATTACHED) {
                let attached = message.params_as::<Attached>().ok_or_else(|| Failure::Other(t!("failure.bridge_not_attached").into()))?;
                check_protocol(&attached)?;
                bridge = Some(attached);
                break;
            }
        }
        wait = opts.step;
    }

    send(&mut writer, &Message::request(SQUADS_ID, rpc::SQUADS, ())).await?;
    let squads = response(&mut lines, SQUADS_ID, wait, "squads").await?.result_as::<Squads>().ok_or_else(|| Failure::Other(t!("failure.squads_unreadable").into()))?;
    let Some(squad) = choose_squad(&squads, opts.prefer.as_deref()) else {
        return Err(Failure::NoSquads.into());
    };

    // The real view says no language (what the daemon speaks in stays) and
    // has no browser panel: the hello asks only to follow.
    let readonly = crate::readonly::on();
    let lang = (!readonly).then(|| crate::i18n::lang().code().to_owned());
    let hello = Message::request(HELLO_ID, rpc::HELLO, HelloParams { version: opts.version.clone(), squad, lang, browser_panel: !readonly });
    if readonly {
        eprintln!("equipe readonly: sent request {} for {}", rpc::HELLO, hello.params.as_ref().and_then(|p| p.get("squad")).and_then(|s| s.as_str()).unwrap_or_default());
    }
    send(&mut writer, &hello).await?;
    let hello = response(&mut lines, HELLO_ID, opts.step, "hello").await?.result_as::<Hello>().ok_or_else(|| Failure::Other(t!("failure.hello_refused").into()))?;
    // What this screen shows the app in, told as it attaches (`screen_look.rs`);
    // the real view tells none. A daemon older than it lets it pass.
    if let Some(look) = crate::screen_look::shown() {
        let _ = send(&mut writer, &Message::notification(rpc::SCREEN_LOOK, look)).await;
    }

    let (events_tx, events) = mpsc::unbounded_channel();
    let pending: Pending = Arc::default();
    let answers = pending.clone();
    tokio::spawn(async move {
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(message) = Message::from_line(&line) else {
                continue;
            };
            if message.method.is_none()
                && let Some(id) = message.id
            {
                // The answer to an `upload` or a `read_upload`.
                if let Some(waiting) = answers.lock().unwrap().remove(&id) {
                    let _ = waiting.send(match (message.error, message.result) {
                        (Some(error), _) => Err(error.message),
                        (None, Some(result)) => Ok(result),
                        (None, None) => Err(t!("failure.answer_unreadable").into()),
                    });
                }
            } else if message.method.as_deref() == Some(rpc::EVENT)
                && let Some(event) = message.params_as::<Event>()
                && events_tx.send(event).is_err()
            {
                break;
            }
        }
        // The connection is over: whoever still waits for an answer is let go.
        answers.lock().unwrap().clear();
    });
    let (commands_tx, mut commands_rx) = mpsc::unbounded_channel::<Message>();
    tokio::spawn(async move {
        while let Some(message) = commands_rx.recv().await {
            if send(&mut writer, &message).await.is_err() {
                break;
            }
        }
        // The window let go: closing our write half is how the daemon (or the
        // bridge) learns this screen left.
        let _ = writer.shutdown().await;
    });
    Ok((
        Greeting {
            bridge,
            squads,
            hello,
        },
        Wire {
            commands: Commands {
                out: commands_tx,
                pending,
                next: Arc::new(AtomicU64::new(FIRST_ASKED_ID)),
            },
            events,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::rpc::SquadEntry;

    fn squads(names: &[&str], last: Option<&str>) -> Squads {
        Squads {
            squads: names.iter().map(|n| SquadEntry { name: (*n).into(), projects: vec![], home: "/h".into() }).collect(),
            last: last.map(Into::into),
        }
    }

    #[test]
    fn picks_the_asked_squad_else_the_last_else_the_first() {
        let s = squads(&["a", "b", "c"], Some("b"));
        assert_eq!(choose_squad(&s, Some("c")).as_deref(), Some("c"));
        assert_eq!(choose_squad(&s, Some("gone")).as_deref(), Some("b"));
        assert_eq!(choose_squad(&s, None).as_deref(), Some("b"));
        assert_eq!(choose_squad(&squads(&["a", "b"], Some("zzz")), None).as_deref(), Some("a"));
        assert_eq!(choose_squad(&squads(&[], None), None), None);
    }

    #[test]
    fn compares_wire_versions() {
        let at = |protocol| Attached { bridge: "b".into(), daemon: Some("d".into()), protocol, started: false };
        assert_eq!(check_protocol(&at(rpc::PROTOCOL)), Ok(()));
        assert_eq!(check_protocol(&at(rpc::PROTOCOL + 1)), Err(Failure::AppTooOld { daemon: rpc::PROTOCOL + 1 }));
        assert_eq!(check_protocol(&at(0)), Err(Failure::DaemonTooOld { daemon: 0 }));
    }

    #[tokio::test]
    async fn the_screen_tells_its_look_as_it_attaches() {
        use equipe_core::protocol::look::ScreenLook;
        crate::screen_look::keep(ScreenLook { machine: "MacBook".into(), app: "equipe-desktop".into(), version: "0.103.25".into(), lang: "en".into(), width: 1200, height: 760, zoom: 100, ..ScreenLook::default() });
        let (ours, theirs) = tokio::io::duplex(1 << 16);
        let (reader, writer) = tokio::io::split(ours);
        let daemon = tokio::spawn(async move {
            let (r, mut w) = tokio::io::split(theirs);
            let mut lines = BufReader::new(r).lines();
            let mut said = Vec::new();
            while let Ok(Some(line)) = lines.next_line().await {
                let message = Message::from_line(&line).unwrap();
                match (message.method.as_deref(), message.id) {
                    (Some(rpc::SQUADS), Some(id)) => w.write_all(Message::response(id, squads(&["acme"], None)).to_line().as_bytes()).await.unwrap(),
                    (Some(rpc::HELLO), Some(id)) => w.write_all(Message::response(id, Hello { version: "d".into(), snapshot: Default::default() }).to_line().as_bytes()).await.unwrap(),
                    (Some(rpc::SCREEN_LOOK), None) => {
                        said.push(message.params_as::<ScreenLook>().expect("a look"));
                        return said;
                    }
                    _ => {}
                }
            }
            said
        });
        let opts = Opts { version: "t".into(), prefer: None, bridged: false, first: Duration::from_secs(2), step: Duration::from_secs(2) };
        let Ok((_greeting, _wire)) = connect(reader, writer, &opts).await else { panic!("not attached") };
        let said = tokio::time::timeout(Duration::from_secs(5), daemon).await.unwrap().unwrap();
        assert_eq!(said.len(), 1, "told right after hello");
        assert!(!said[0].machine.is_empty() && !said[0].lang.is_empty());
    }

    #[tokio::test]
    async fn an_empty_squad_list_is_its_own_cause() {
        let (ours, theirs) = tokio::io::duplex(4096);
        let (reader, writer) = tokio::io::split(ours);
        tokio::spawn(async move {
            let (r, mut w) = tokio::io::split(theirs);
            let mut lines = BufReader::new(r).lines();
            let request = Message::from_line(&lines.next_line().await.unwrap().unwrap()).unwrap();
            w.write_all(Message::response(request.id.unwrap(), squads(&[], None)).to_line().as_bytes()).await.unwrap();
            let _ = lines.next_line().await;
        });
        let opts = Opts { version: "t".into(), prefer: None, bridged: false, first: Duration::from_secs(2), step: Duration::from_secs(2) };
        assert!(matches!(connect(reader, writer, &opts).await, Err(Stop::Failed(Failure::NoSquads))));
    }

    #[tokio::test]
    async fn a_bridge_that_hangs_up_is_eof_and_a_silent_one_times_out() {
        let opts = Opts { version: "t".into(), prefer: None, bridged: true, first: Duration::from_millis(100), step: Duration::from_millis(100) };
        let (ours, theirs) = tokio::io::duplex(64);
        drop(theirs);
        let (r, w) = tokio::io::split(ours);
        assert!(matches!(connect(r, w, &opts).await, Err(Stop::Eof)));

        let (ours, _quiet) = tokio::io::duplex(64);
        let (r, w) = tokio::io::split(ours);
        assert!(matches!(connect(r, w, &opts).await, Err(Stop::Failed(Failure::NoAnswer))));
    }
}
