//! The Screen panel's line to another machine's screen: `ssh <host> equipe
//! screen --stdio` over the window's shared SSH connection (no new sign-in, no
//! port of its own), which bridges to the VNC server on that machine. The
//! bytes are RFB (`equipe_rfb`): this module reads them on a thread of its
//! own, keeps the framebuffer, and hands the window the latest picture —
//! only once it has drawn the one before, so a slow line shows fewer, newer
//! pictures rather than a queue of old ones.
//!
//! Over a long round trip the picture keeps up because the server is asked to
//! push changes as they happen (continuous updates, with fences answered),
//! falling back to asking again after each update for servers without them.
//! See docs/DESKTOP.md, "The Screen panel".

use std::io::{self, BufRead as _, BufReader, Read, Write};
use std::process::{Child, ChildStdout, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use equipe_rfb::wire;
use equipe_rfb::{Decoder, Event, PixelFormat};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::ssh::Launcher;
use crate::target::Endpoint;

/// What the bridge said first, when there is a screen.
#[derive(Debug, Clone, PartialEq)]
pub struct Hello {
    pub width: u16,
    pub height: u16,
    pub name: String,
    /// The far machine's OS (`linux`, `macos`): which key ⌘ becomes there.
    pub os: String,
}

/// Why the line ended.
#[derive(Debug, Clone, PartialEq)]
pub enum End {
    /// The far machine has no screen equipe can show: the bridge's kind
    /// (`no_server`, `needs_password`, `auth_failed`, `unsupported`, …) and its words.
    NoScreen { kind: String, detail: String },
    /// The `equipe` there predates `equipe screen`.
    Outdated,
    /// No `equipe` on that machine (or not on the PATH SSH gives).
    NoEquipe,
    /// `ssh` itself ended (sign-in, host key, network): its last words.
    Ssh(String),
    /// It was showing, and the line dropped.
    Lost(String),
}

impl End {
    /// Worth trying again on its own, a little later.
    pub fn retries(&self) -> bool {
        matches!(self, End::Lost(_))
    }
}

/// What the line tells the panel.
#[derive(Debug)]
pub enum Update {
    Ready(Hello),
    /// A new picture waits in [`Latest`].
    Picture,
    /// The far screen changed size.
    Resized(u16, u16),
    Ended(End),
}

/// A picture of the far screen: BGRA rows, as the window draws them.
pub struct Frame {
    pub width: u16,
    pub height: u16,
    pub data: Vec<u8>,
}

/// The newest picture, handed from the reader to the window.
pub struct Latest {
    frame: Mutex<Option<Frame>>,
    /// The window drew the last picture and wants the next.
    wanted: AtomicBool,
}

impl Default for Latest {
    /// The window wants the first picture from the start.
    fn default() -> Self {
        Latest { frame: Mutex::new(None), wanted: AtomicBool::new(true) }
    }
}

impl Latest {
    /// The window has drawn what it had: the next picture, if one is waiting
    /// already (else the reader says when it comes).
    pub fn next(&self) -> Option<Frame> {
        self.wanted.store(true, Ordering::SeqCst);
        if self.frame.lock().unwrap().is_some() && self.wanted.swap(false, Ordering::SeqCst) {
            return self.frame.lock().unwrap().take();
        }
        None
    }

    /// The picture the reader said is waiting.
    pub fn take(&self) -> Option<Frame> {
        self.frame.lock().unwrap().take()
    }

    /// The reader's side: keep `frame` as the newest; whether to tell the window.
    fn put(&self, frame: Frame) -> bool {
        *self.frame.lock().unwrap() = Some(frame);
        self.wanted.swap(false, Ordering::SeqCst)
    }
}

/// Where the screen comes from.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    Ssh(crate::target::SshHost),
    /// `$EQUIPE_SCREEN_STDIO`: a command run here instead (`sh -c`), for
    /// trying the panel against a VNC server without another machine.
    Shell(String),
}

/// The screen to show for the window's daemon: the far machine's, over SSH.
/// This Mac's own screen is not shown in a window on it.
pub fn source(endpoint: &Endpoint) -> Option<Source> {
    if let Ok(command) = std::env::var("EQUIPE_SCREEN_STDIO")
        && !command.trim().is_empty()
    {
        return Some(Source::Shell(command));
    }
    match endpoint {
        Endpoint::Ssh(host) => Some(Source::Ssh(host.clone())),
        Endpoint::Local { .. } => None,
    }
}

/// The panel's end of the line. Dropping it ends `ssh`.
pub struct Session {
    to: mpsc::Sender<Vec<u8>>,
    pub latest: Arc<Latest>,
    child: Arc<Mutex<Option<Child>>>,
}

impl Session {
    pub fn pointer(&self, buttons: u8, x: u16, y: u16) {
        let _ = self.to.send(wire::pointer_event(buttons, x, y).to_vec());
    }

    pub fn key(&self, down: bool, keysym: u32) {
        let _ = self.to.send(wire::key_event(down, keysym).to_vec());
    }

    /// Typed text, as the characters themselves: each pressed and let go.
    pub fn text(&self, text: &str) {
        let mut bytes = Vec::with_capacity(text.len() * 16);
        for c in text.chars() {
            let keysym = equipe_rfb::keysym::of_char(c);
            bytes.extend_from_slice(&wire::key_event(true, keysym));
            bytes.extend_from_slice(&wire::key_event(false, keysym));
        }
        let _ = self.to.send(bytes);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// JPEG quality and zlib effort to ask for: lighter over a network.
pub fn quality(source: &Source) -> (u8, u8) {
    match source {
        Source::Ssh(_) => (6, 2),
        Source::Shell(_) => (8, 1),
    }
}

/// Start the line on a thread of its own.
pub fn spawn(source: Source) -> (Session, UnboundedReceiver<Update>) {
    let (to_tx, to_rx) = mpsc::channel::<Vec<u8>>();
    let (up_tx, up_rx) = unbounded_channel();
    let latest = Arc::new(Latest::default());
    let child = Arc::new(Mutex::new(None));
    let session = Session { to: to_tx.clone(), latest: latest.clone(), child: child.clone() };
    std::thread::Builder::new()
        .name("equipe-screen".into())
        .spawn(move || {
            let end = run(&source, to_tx, to_rx, &up_tx, &latest, &child);
            let _ = up_tx.send(Update::Ended(end));
        })
        .expect("spawn the screen thread");
    (session, up_rx)
}

fn command(source: &Source) -> io::Result<Child> {
    match source {
        Source::Ssh(host) => Launcher::system(None).screen(host),
        Source::Shell(line) => std::process::Command::new("/bin/sh").arg("-c").arg(line).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn(),
    }
}

/// Both halves of the child's pipes, for the handshake.
struct Duplex<'a, R, W> {
    read: &'a mut R,
    write: &'a mut W,
}

impl<R: Read, W> Read for Duplex<'_, R, W> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.read.read(buf)
    }
}

impl<R, W: Write> Write for Duplex<'_, R, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.write.flush()
    }
}

/// What the bridge's first line says.
pub fn parse_hello(line: &str) -> Result<Hello, End> {
    let value: serde_json::Value = serde_json::from_str(line.trim()).map_err(|_| End::Lost(format!("not a screen bridge: {}", line.trim())))?;
    if value.get("screen").is_none() {
        return Err(End::Lost("not a screen bridge".into()));
    }
    let text = |key: &str| value.get(key).and_then(serde_json::Value::as_str).unwrap_or_default().to_owned();
    if let Some(kind) = value.get("error").and_then(serde_json::Value::as_str) {
        return Err(End::NoScreen { kind: kind.to_owned(), detail: text("detail") });
    }
    let size = |key: &str| value.get(key).and_then(serde_json::Value::as_u64).unwrap_or(0) as u16;
    Ok(Hello { width: size("width"), height: size("height"), name: text("name"), os: text("os") })
}

/// How `ssh` ended, once its output stopped before any first line.
pub fn classify(code: Option<i32>, stderr: &[String]) -> End {
    let said = stderr.iter().rev().find(|l| !l.trim().is_empty()).cloned().unwrap_or_default();
    if stderr.iter().any(|l| l.contains("unrecognized subcommand") || l.contains("unexpected argument '--stdio'")) {
        return End::Outdated;
    }
    match code {
        Some(127) => End::NoEquipe,
        Some(255) => End::Ssh(said),
        _ => End::Lost(said),
    }
}

fn run(source: &Source, to_tx: mpsc::Sender<Vec<u8>>, to_rx: mpsc::Receiver<Vec<u8>>, updates: &UnboundedSender<Update>, latest: &Latest, slot: &Mutex<Option<Child>>) -> End {
    if crate::readonly::blocked("the remote screen") {
        return End::Lost("read-only view".into());
    }
    let mut child = match command(source) {
        Ok(child) => child,
        Err(e) => return End::Ssh(e.to_string()),
    };
    let (Some(stdout), Some(mut stdin), stderr) = (child.stdout.take(), child.stdin.take(), child.stderr.take()) else {
        return End::Ssh("no pipes to ssh".into());
    };
    let said = Arc::new(Mutex::new(Vec::<String>::new()));
    if let Some(stderr) = stderr {
        let said = said.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let mut kept = said.lock().unwrap();
                if kept.len() >= 20 {
                    kept.remove(0);
                }
                kept.push(line);
            }
        });
    }
    *slot.lock().unwrap() = Some(child);
    let ended = |slot: &Mutex<Option<Child>>| -> Option<i32> {
        let mut guard = slot.lock().unwrap();
        let child = guard.as_mut()?;
        for _ in 0..30 {
            if let Ok(Some(status)) = child.try_wait() {
                return status.code();
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        None
    };
    let mut reader = BufReader::with_capacity(256 * 1024, stdout);

    // The bridge's first line: a screen, or why there is none.
    let mut first = Vec::new();
    match (&mut reader).take(64 * 1024).read_until(b'\n', &mut first) {
        Ok(n) if n > 0 && first.ends_with(b"\n") => {}
        _ => {
            let code = ended(slot);
            // Its last words may still be on their way.
            std::thread::sleep(Duration::from_millis(150));
            return classify(code, &said.lock().unwrap());
        }
    }
    let hello = match parse_hello(&String::from_utf8_lossy(&first)) {
        Ok(hello) => hello,
        Err(end) => return end,
    };

    let init = match wire::client_handshake(&mut Duplex { read: &mut reader, write: &mut stdin }, None, true) {
        Ok(init) => init,
        Err(e) => return End::Lost(e.to_string()),
    };
    let (q, c) = quality(source);
    let mut setup = wire::set_pixel_format(&PixelFormat::BGRX);
    setup.extend(wire::set_encodings(&Decoder::encodings(q, c)));
    setup.extend(wire::update_request(false, 0, 0, init.width, init.height));
    if stdin.write_all(&setup).and_then(|()| stdin.flush()).is_err() {
        return End::Lost("the line closed".into());
    }
    let _ = updates.send(Update::Ready(Hello { width: init.width, height: init.height, name: init.name.clone(), ..hello }));

    // Everything to the far end goes through one writer, in order.
    std::thread::Builder::new()
        .name("equipe-screen-out".into())
        .spawn(move || {
            while let Ok(first) = to_rx.recv() {
                let mut batch = first;
                // What queued meanwhile goes in the same write.
                while let Ok(more) = to_rx.try_recv() {
                    batch.extend(more);
                }
                if stdin.write_all(&batch).and_then(|()| stdin.flush()).is_err() {
                    break;
                }
            }
        })
        .expect("spawn the screen writer");

    let end = follow(&mut reader, init.width, init.height, &to_tx, updates, latest);
    match end {
        // The far side went away: say why, if ssh did.
        End::Lost(why) => {
            let code = ended(slot);
            match classify(code, &said.lock().unwrap()) {
                End::Ssh(s) if !s.is_empty() => End::Ssh(s),
                _ => End::Lost(why),
            }
        }
        other => other,
    }
}

/// Read the server's messages until the line ends.
fn follow(reader: &mut BufReader<ChildStdout>, width: u16, height: u16, to: &mpsc::Sender<Vec<u8>>, updates: &UnboundedSender<Update>, latest: &Latest) -> End {
    let mut decoder = Decoder::new(width, height);
    // Pushed by the server once it said it can (continuous updates); asked for after each update until then.
    let mut continuous = false;
    loop {
        let event = match decoder.read_message(reader) {
            Ok(event) => event,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return End::Lost("the line closed".into()),
            Err(e) => return End::Lost(e.to_string()),
        };
        match event {
            Event::Update { damage, resized } => {
                let fb = decoder.framebuffer();
                if resized {
                    let _ = updates.send(Update::Resized(fb.width, fb.height));
                    if continuous {
                        let _ = to.send(wire::enable_continuous_updates(true, 0, 0, fb.width, fb.height).to_vec());
                    }
                }
                if !continuous {
                    let _ = to.send(wire::update_request(!resized, 0, 0, fb.width, fb.height).to_vec());
                }
                if damage.is_empty() && !resized {
                    continue;
                }
                let frame = Frame { width: fb.width, height: fb.height, data: fb.data.clone() };
                if latest.put(frame) && updates.send(Update::Picture).is_err() {
                    return End::Lost("the panel closed".into());
                }
            }
            // The server speaks continuous updates: from now on it pushes.
            Event::EndOfContinuousUpdates if !continuous => {
                continuous = true;
                let fb = decoder.framebuffer();
                let _ = to.send(wire::enable_continuous_updates(true, 0, 0, fb.width, fb.height).to_vec());
            }
            Event::Fence { flags, payload } => {
                if let Some(reply) = wire::fence_reply(flags, &payload) {
                    let _ = to.send(reply);
                }
            }
            _ => {}
        }
        if updates.is_closed() {
            return End::Lost("the panel closed".into());
        }
    }
}

/// The encodings asked for.
#[cfg(test)]
fn asked(source: &Source) -> Vec<i32> {
    let (q, c) = quality(source);
    Decoder::encodings(q, c)
}

/// Whether `encodings` lets the server send compact pictures (not only Raw).
#[cfg(test)]
fn compact(encodings: &[i32]) -> bool {
    use equipe_rfb::wire::encoding;
    encodings.contains(&encoding::TIGHT) && encodings.contains(&encoding::ZRLE) && encodings.contains(&encoding::COPY_RECT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_line_says_there_is_a_screen_or_why_not() {
        let hello = parse_hello(r#"{"screen":1,"ok":true,"os":"linux","width":1440,"height":900,"name":"tenki:1","server":"unix:/x"}"#).unwrap();
        assert_eq!(hello, Hello { width: 1440, height: 900, name: "tenki:1".into(), os: "linux".into() });
        assert_eq!(
            parse_hello(r#"{"screen":1,"error":"no_server","detail":"no VNC server answers at 127.0.0.1:5900"}"#),
            Err(End::NoScreen { kind: "no_server".into(), detail: "no VNC server answers at 127.0.0.1:5900".into() })
        );
        assert!(matches!(parse_hello("{\"jsonrpc\":\"2.0\"}"), Err(End::Lost(_))));
        assert!(matches!(parse_hello("RFB 003.008"), Err(End::Lost(_))));
    }

    #[test]
    fn an_old_equipe_or_none_at_all_is_told_apart_from_ssh() {
        let lines = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(classify(Some(2), &lines(&["error: unrecognized subcommand 'screen'", "", "Usage: equipe [COMMAND]"])), End::Outdated);
        assert_eq!(classify(Some(127), &lines(&["sh: 1: exec: equipe: not found"])), End::NoEquipe);
        assert_eq!(classify(Some(255), &lines(&["Permission denied (publickey)."])), End::Ssh("Permission denied (publickey).".into()));
        assert_eq!(classify(Some(1), &lines(&["boom"])), End::Lost("boom".into()));
        assert!(End::Lost(String::new()).retries());
        assert!(!End::NoScreen { kind: "no_server".into(), detail: String::new() }.retries());
    }

    #[test]
    fn the_window_gets_the_newest_picture_once_it_drew_the_last() {
        let latest = Latest::default();
        let frame = |n: u8| Frame { width: 1, height: 1, data: vec![n, 0, 0, 255] };
        // The first is told at once.
        assert!(latest.put(frame(0)));
        assert_eq!(latest.take().unwrap().data[0], 0);
        // While it is drawn, the next is kept, not told.
        assert!(!latest.put(frame(1)));
        // Drawn: the window finds the one waiting itself.
        assert_eq!(latest.next().unwrap().data[0], 1);
        // Ready and nothing waiting: the next is told...
        assert!(latest.next().is_none());
        assert!(latest.put(frame(2)));
        // ...and while that one is drawn, newer ones replace each other.
        assert!(!latest.put(frame(3)));
        assert!(!latest.put(frame(4)));
        assert_eq!(latest.take().unwrap().data[0], 4);
    }

    #[test]
    fn a_far_screen_is_asked_for_compactly() {
        let ssh = Source::Ssh(crate::target::SshHost::parse("me@far").unwrap());
        assert!(compact(&asked(&ssh)));
        assert!(asked(&ssh).contains(&equipe_rfb::wire::encoding::CONTINUOUS_UPDATES) && asked(&ssh).contains(&equipe_rfb::wire::encoding::FENCE));
        assert_eq!(quality(&ssh), (6, 2));
        assert_eq!(source(&Endpoint::Local { socket: "/x".into() }).filter(|_| std::env::var("EQUIPE_SCREEN_STDIO").is_err()), None, "this Mac's own screen isn't shown on it");
    }

    /// The whole line, through a stand-in bridge: the first line, the
    /// handshake, a picture, then input going back.
    #[test]
    fn a_stand_in_bridge_shows_a_picture_and_takes_input() {
        let dir = std::env::temp_dir().join(format!("eq-screen-line-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // The server's bytes: the first line, a 3.8 None handshake, ServerInit
        // (2×1), then an update filling it red with Tight.
        let mut said = br#"{"screen":1,"ok":true,"os":"linux","width":2,"height":1,"name":"t"}"#.to_vec();
        said.push(b'\n');
        said.extend(b"RFB 003.008\n");
        said.extend([1, 1]);
        said.extend(0u32.to_be_bytes());
        said.extend(equipe_rfb::ServerInit { width: 2, height: 1, format: PixelFormat::BGRX, name: "t".into() }.to_bytes());
        said.extend([0, 0, 0, 1, 0, 0, 0, 0, 0, 2, 0, 1, 0, 0, 0, 7, 0x80, 255, 0, 0]);
        std::fs::write(dir.join("out"), &said).unwrap();
        // It answers, then keeps what the app writes until its input closes.
        let line = format!("cat '{}'; cat > '{}'", dir.join("out").display(), dir.join("in").display());
        let (session, mut updates) = spawn(Source::Shell(line));
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            let next = async |updates: &mut UnboundedReceiver<Update>| tokio::time::timeout(Duration::from_secs(5), updates.recv()).await.unwrap().unwrap();
            assert!(matches!(next(&mut updates).await, Update::Ready(Hello { width: 2, height: 1, .. })));
            // The first picture is kept until the window asks.
            loop {
                if let Some(frame) = session.latest.next() {
                    assert_eq!(&frame.data[..4], &[0, 0, 255, 255], "red, in BGRA");
                    break;
                }
                match next(&mut updates).await {
                    Update::Picture => {}
                    Update::Ended(end) => panic!("{end:?}"),
                    _ => {}
                }
            }
        });
        session.text("한a");
        session.pointer(wire::button::LEFT, 1, 0);
        std::thread::sleep(Duration::from_millis(300));
        drop(session);
        let wrote = std::fs::read(dir.join("in")).unwrap();
        // After the handshake (version, None, shared) and the setup, the input.
        assert!(wrote.starts_with(b"RFB 003.008\n\x01\x01"));
        let tail = &wrote[wrote.len() - (8 * 4 + 6)..];
        assert_eq!(&tail[..8], &wire::key_event(true, 0x0100_d55c), "the syllable, whole");
        assert_eq!(&tail[24..32], &wire::key_event(false, 'a' as u32));
        assert_eq!(&tail[32..], &wire::pointer_event(1, 1, 0));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
