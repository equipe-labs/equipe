//! Files a phone attaches, received in pieces (`upload`), each piece one sealed
//! frame — see "Attachments" in `docs/remote-protocol.md`. Pieces wait in
//! `uploads/.partial/` by the phone's `upload_id`, so a connection that drops
//! halfway loses nothing; the whole file is then kept as a screen's upload
//! (`equipe_core::gateway::create_upload`) and tagged as the desktop tags it,
//! with a sound's transcript beside it and a video's frames made here
//! (`ffmpeg`), since a member can neither hear nor watch.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use equipe_core::gateway;
use equipe_core::protocol::rpc::{Message, UploadKind};
use serde::Deserialize;
use serde_json::{Value, json};

/// The most one piece carries, decoded: sealed and base64'd twice it stays
/// near 700 KB, well inside the relay's 1 MiB frame.
pub const CHUNK_MAX: usize = 384 * 1024;
pub const IMAGE_MAX: u64 = 8 << 20;
pub const FILE_MAX: u64 = 25 << 20;
pub const VIDEO_MAX: u64 = 64 << 20;
/// A transcript rides with the first piece and both must fit in one frame;
/// five minutes of speech is some 15 KB.
pub const TRANSCRIPT_MAX: usize = 64 * 1024;
/// Files under way at once from one device.
const AT_ONCE: usize = 4;
/// A file no piece has reached for this long is thrown away; a finished one's
/// answer is given again for as long.
const IDLE_MS: u64 = 60 * 60 * 1000;
/// Text files up to this size get their lines counted.
const LINES_MAX: u64 = 8 << 20;
/// A frame's longest side, as a member can use it (the desktop's `LONG_SIDE`).
const LONG_SIDE: u32 = 1568;
const SOUNDS: [&str; 8] = ["m4a", "mp3", "wav", "aac", "caf", "ogg", "opus", "flac"];

/// What a device may learn of the limits, in its `hello` answer.
pub fn limits() -> Value {
    json!({ "chunk": CHUNK_MAX, "image": IMAGE_MAX, "file": FILE_MAX, "video": VIDEO_MAX, "transcript": TRANSCRIPT_MAX })
}

/// One piece, as the phone sends it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Piece {
    pub upload_id: String,
    pub name: String,
    pub kind: UploadKind,
    pub total: u64,
    pub offset: u64,
    pub data: String,
    #[serde(default)]
    pub transcript: Option<String>,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub pages: Option<u32>,
}

/// A refusal: the code, the message and what the phone may need to go on.
#[derive(Debug, Clone, PartialEq)]
pub struct Refusal {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

impl Refusal {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), data: None }
    }

    fn with(code: i64, message: impl Into<String>, data: Value) -> Self {
        Self { code, message: message.into(), data: Some(data) }
    }

    pub fn answer(&self, id: u64) -> Message {
        match &self.data {
            Some(data) => Message::failure_with(id, self.code, &self.message, data),
            None => Message::failure(id, self.code, &self.message),
        }
    }
}

#[derive(Debug)]
struct Partial {
    name: String,
    kind: UploadKind,
    total: u64,
    received: u64,
    path: PathBuf,
    touched: u64,
    transcript: Option<String>,
    duration: Option<f64>,
    pages: Option<u32>,
}

/// Every device's files under way, and the answers to those just finished.
#[derive(Debug)]
pub struct Uploads {
    home: PathBuf,
    /// Where `ffmpeg` and `ffprobe` are looked for.
    tools: Vec<PathBuf>,
    partial: HashMap<(String, String), Partial>,
    done: HashMap<(String, String), (Value, u64)>,
}

impl Uploads {
    pub fn new(home: &Path) -> Self {
        let tools = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].iter().map(PathBuf::from).collect();
        Self::with_tools(home, tools)
    }

    pub fn with_tools(home: &Path, tools: Vec<PathBuf>) -> Self {
        // Pieces left by a daemon before this one can't be resumed: it forgot their ids.
        let _ = std::fs::remove_dir_all(partial_dir(home));
        Self { home: home.to_owned(), tools, partial: HashMap::new(), done: HashMap::new() }
    }

    /// Take one piece from `device`: how far the file is, and — with its
    /// last piece — where it was kept and its tag.
    pub fn piece(&mut self, device: &str, piece: Piece, now: u64) -> Result<Value, Refusal> {
        use base64::Engine as _;
        self.sweep(now);
        if !valid_id(&piece.upload_id) {
            return Err(Refusal::new(-32602, "upload_id must be 8 to 64 of A-Z a-z 0-9 - _"));
        }
        let key = (device.to_owned(), piece.upload_id.clone());
        if let Some((answer, _)) = self.done.get(&key) {
            return Ok(answer.clone());
        }
        let limit = match piece.kind {
            UploadKind::Image => IMAGE_MAX,
            UploadKind::Video => VIDEO_MAX,
            UploadKind::Pdf | UploadKind::File => FILE_MAX,
        };
        if piece.total > limit {
            return Err(Refusal::with(-32602, "too large", json!({ "limit": limit })));
        }
        if piece.transcript.as_ref().is_some_and(|t| t.len() > TRANSCRIPT_MAX) {
            return Err(Refusal::with(-32602, "too large", json!({ "limit": TRANSCRIPT_MAX })));
        }
        // Checked before anything is decoded: base64 of CHUNK_MAX, padded.
        if piece.data.len() > CHUNK_MAX.div_ceil(3) * 4 {
            return Err(Refusal::with(-32602, "piece too large", json!({ "limit": CHUNK_MAX })));
        }
        let data = base64::engine::general_purpose::STANDARD.decode(piece.data.trim()).map_err(|_| Refusal::new(-32602, "data isn't base64"))?;
        if data.len() > CHUNK_MAX || piece.offset + data.len() as u64 > piece.total {
            return Err(Refusal::with(-32602, "piece too large", json!({ "limit": CHUNK_MAX })));
        }
        let received = match self.partial.get(&key) {
            Some(p) if p.name != piece.name || p.kind != piece.kind || p.total != piece.total => return Err(Refusal::new(-32602, "not the same upload")),
            Some(p) => p.received,
            None => 0,
        };
        if piece.offset < received {
            // Had already: said again after a lost answer.
            self.partial.get_mut(&key).expect("known").touched = now;
            return Ok(json!({ "received": received }));
        }
        if piece.offset > received {
            return Err(Refusal::with(-32602, "send from received", json!({ "received": received })));
        }
        if !self.partial.contains_key(&key) {
            gateway::upload_name(&piece.name, Some(piece.kind)).map_err(|why| Refusal::new(-32602, why))?;
            if self.partial.keys().filter(|(d, _)| d == device).count() >= AT_ONCE {
                return Err(Refusal::new(-32000, "too many uploads at once"));
            }
            let path = self.start(device, &piece.upload_id).map_err(|e| Refusal::new(-32000, e))?;
            self.partial.insert(
                key.clone(),
                Partial { name: piece.name.clone(), kind: piece.kind, total: piece.total, received: 0, path, touched: now, transcript: piece.transcript.clone(), duration: piece.duration, pages: piece.pages },
            );
        }
        let partial = self.partial.get_mut(&key).expect("started");
        let written = std::fs::OpenOptions::new().append(true).open(&partial.path).and_then(|mut f| f.write_all(&data));
        if let Err(e) = written {
            let gone = self.partial.remove(&key).expect("started");
            let _ = std::fs::remove_file(&gone.path);
            return Err(Refusal::new(-32000, e.to_string()));
        }
        partial.received += data.len() as u64;
        partial.touched = now;
        if partial.received < partial.total {
            return Ok(json!({ "received": partial.received }));
        }
        let partial = self.partial.remove(&key).expect("started");
        let answer = self.finish(&partial).map_err(|e| {
            let _ = std::fs::remove_file(&partial.path);
            Refusal::new(-32000, e)
        })?;
        self.done.insert(key, (answer.clone(), now));
        Ok(answer)
    }

    /// Throw away what came of `upload_id`.
    pub fn cancel(&mut self, device: &str, upload_id: &str) {
        if let Some(partial) = self.partial.remove(&(device.to_owned(), upload_id.to_owned())) {
            let _ = std::fs::remove_file(partial.path);
        }
    }

    /// Files no piece has reached for an hour go; so do old answers.
    pub fn sweep(&mut self, now: u64) {
        self.partial.retain(|_, p| {
            let keep = now.saturating_sub(p.touched) < IDLE_MS;
            if !keep {
                let _ = std::fs::remove_file(&p.path);
            }
            keep
        });
        self.done.retain(|_, (_, at)| now.saturating_sub(*at) < IDLE_MS);
    }

    fn start(&self, device: &str, upload_id: &str) -> Result<PathBuf, String> {
        use std::os::unix::fs::OpenOptionsExt as _;
        let dir = partial_dir(&self.home);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        restrict(&gateway::uploads_dir(&self.home))?;
        restrict(&dir)?;
        let path = dir.join(format!("{device}-{upload_id}"));
        std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path).map_err(|e| e.to_string())?;
        Ok(path)
    }

    /// The whole file is in: keep it where a screen's would be, and tag it.
    fn finish(&self, partial: &Partial) -> Result<Value, String> {
        let (file, path) = gateway::create_upload(&self.home, &partial.name, Some(partial.kind))?;
        drop(file);
        std::fs::rename(&partial.path, &path).map_err(|e| e.to_string())?;
        let shown = path.to_string_lossy().into_owned();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_owned();
        let stem = Path::new(&partial.name).file_stem().and_then(|s| s.to_str()).unwrap_or("upload").to_owned();
        let mut tags = Vec::new();
        let mut notice = None;
        match partial.kind {
            UploadKind::Image => tags.push(format!("[image: {shown}]")),
            UploadKind::Pdf => tags.push(match partial.pages {
                Some(1) => format!("[pdf: {shown} · 1 page]"),
                Some(n) => format!("[pdf: {shown} · {n} pages]"),
                None => format!("[pdf: {shown}]"),
            }),
            UploadKind::File if SOUNDS.contains(&ext.as_str()) => {
                let duration = partial.duration.or_else(|| self.probe(&path).and_then(|p| p.duration));
                tags.push(match duration {
                    Some(d) => format!("[audio: {shown} · {}]", clock(d)),
                    None => format!("[audio: {shown}]"),
                });
            }
            UploadKind::File => tags.push(match lines(&path) {
                Some(1) => format!("[file: {shown} · 1 line]"),
                Some(n) => format!("[file: {shown} · {n} lines]"),
                None => format!("[file: {shown}]"),
            }),
            UploadKind::Video => match self.frames(&path, &stem, partial.duration) {
                Ok(video) => {
                    let mut parts = vec![shown.clone(), clock(video.duration)];
                    if let Some((w, h)) = video.dims {
                        parts.push(format!("{w}×{h}"));
                    }
                    if !video.frames.is_empty() {
                        let list: Vec<String> = video.frames.iter().map(|(at, p)| format!("{}@{}", p.display(), clock(*at))).collect();
                        parts.push(format!("frames: {}", list.join(", ")));
                    }
                    tags.push(format!("[video: {}]", parts.join(" · ")));
                }
                Err(why) => {
                    if why == "no_ffmpeg" {
                        equipe_core::daemon_log!("remote: a phone sent a video, but ffmpeg isn't installed (brew install ffmpeg): kept without frames");
                    } else {
                        equipe_core::daemon_log!("remote: a phone's video gave no frames: {why}");
                    }
                    notice = Some(if why == "no_ffmpeg" { "no_ffmpeg" } else { "no_frames" });
                    tags.push(match partial.duration {
                        Some(d) => format!("[video: {shown} · {}]", clock(d)),
                        None => format!("[video: {shown}]"),
                    });
                }
            },
        }
        let heard = matches!(partial.kind, UploadKind::Video) || SOUNDS.contains(&ext.as_str());
        if let Some(text) = partial.transcript.as_deref().map(str::trim).filter(|t| heard && !t.is_empty()) {
            let (mut file, at) = gateway::create_upload(&self.home, &format!("{stem}-transcript.txt"), Some(UploadKind::File))?;
            file.write_all(text.as_bytes()).and_then(|_| file.write_all(b"\n")).map_err(|e| e.to_string())?;
            let n = text.lines().count();
            tags.push(format!("[file: {} · {n} {}]", at.display(), if n == 1 { "line" } else { "lines" }));
        }
        let mut answer = json!({ "received": partial.total, "path": shown, "tag": tags.join(" ") });
        if let Some(notice) = notice {
            answer["notice"] = json!(notice);
        }
        Ok(answer)
    }

    fn tool(&self, name: &str) -> Option<PathBuf> {
        self.tools.iter().map(|dir| dir.join(name)).find(|p| p.is_file())
    }

    fn probe(&self, path: &Path) -> Option<Probe> {
        let out = Command::new(self.tool("ffprobe")?)
            .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height:format=duration", "-of", "default=nw=1"])
            .arg(path)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let field = |key: &str| text.lines().find_map(|l| l.strip_prefix(key).and_then(|v| v.strip_prefix('='))).map(str::trim);
        let dims = field("width").and_then(|w| w.parse().ok()).zip(field("height").and_then(|h| h.parse().ok()));
        let duration = field("duration").and_then(|d| d.parse::<f64>().ok()).filter(|d| d.is_finite() && *d > 0.);
        Some(Probe { duration, dims })
    }

    /// A video's frames, each kept as an image upload beside it.
    fn frames(&self, path: &Path, stem: &str, said: Option<f64>) -> Result<Video, String> {
        let ffmpeg = self.tool("ffmpeg").ok_or("no_ffmpeg")?;
        let probe = self.probe(path).ok_or("ffprobe couldn't read it")?;
        let duration = probe.duration.or(said).ok_or("no length")?;
        let mut frames = Vec::new();
        for (i, at) in frame_times(duration).into_iter().enumerate() {
            let (file, out) = gateway::create_upload(&self.home, &format!("{stem}-frame-{}.jpg", i + 1), Some(UploadKind::Image))?;
            drop(file);
            let made = Command::new(&ffmpeg)
                .args(["-v", "error", "-y", "-ss", &format!("{at:.3}"), "-i"])
                .arg(path)
                .args(["-frames:v", "1", "-vf", &format!("scale='if(gt(iw,ih),min({LONG_SIDE},iw),-2)':'if(gt(iw,ih),-2,min({LONG_SIDE},ih))'"), "-q:v", "3", "-f", "image2"])
                .arg(&out)
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .status();
            if made.is_ok_and(|s| s.success()) && std::fs::metadata(&out).is_ok_and(|m| m.len() > 0) {
                frames.push((at, out));
            } else {
                let _ = std::fs::remove_file(&out);
            }
        }
        if frames.is_empty() {
            return Err("no frame could be read".into());
        }
        Ok(Video { duration, dims: probe.dims, frames })
    }
}

struct Probe {
    duration: Option<f64>,
    dims: Option<(u32, u32)>,
}

struct Video {
    duration: f64,
    dims: Option<(u32, u32)>,
    frames: Vec<(f64, PathBuf)>,
}

fn partial_dir(home: &Path) -> PathBuf {
    gateway::uploads_dir(home).join(".partial")
}

fn restrict(dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())
}

fn valid_id(id: &str) -> bool {
    (8..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// A text file's lines; `None` when it isn't UTF-8 text or is too big to count.
fn lines(path: &Path) -> Option<usize> {
    let mut bytes = Vec::new();
    File::open(path).ok()?.take(LINES_MAX + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > LINES_MAX || bytes.contains(&0) {
        return None;
    }
    let text = std::str::from_utf8(&bytes).ok()?;
    Some(text.lines().count())
}

/// How many frames a video of `duration` seconds carries — one per ten
/// seconds, 4 to 12 — and when: the middles of equal parts (the desktop's
/// `media::frame_times`).
pub fn frame_times(duration: f64) -> Vec<f64> {
    let n = ((duration / 10.).round().max(0.) as usize).clamp(4, 12);
    (0..n).map(|i| duration * (i as f64 + 0.5) / n as f64).collect()
}

/// `0:03`, `1:42`, `1:02:03`.
pub fn clock(seconds: f64) -> String {
    let s = seconds.max(0.).round() as u64;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

#[cfg(test)]
mod tests {
    use super::super::core::tests::{Phone, pair};
    use super::super::core::{Action, Core};
    use super::super::{Config, seal};
    use super::*;
    use base64::Engine as _;
    use equipe_core::protocol::Command;
    use equipe_core::protocol::tags::{self, TagKind};

    struct Rig {
        home: PathBuf,
        core: Core,
        uploads: Uploads,
        phone: Phone,
        config: Config,
        now: u64,
        next: u64,
    }

    impl Rig {
        fn new(tag: &str, tools: Vec<PathBuf>) -> Self {
            let home = std::env::temp_dir().join(format!("equipe-phone-uploads-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&home);
            std::fs::create_dir_all(&home).unwrap();
            let keys = seal::generate();
            let config = Config { relay: "https://relay.test".into(), id: "d1".into(), token: "t".into(), private: keys.private, public: keys.public, devices: vec![] };
            let mut core = Core::new(Some(config));
            core.squads = vec![("acme".into(), vec!["site".into()])];
            let mut phone = Phone::new();
            let now = super::super::now_ms();
            pair(&mut core, &mut phone, now);
            core.on_relay(&json!({ "type": "presence", "device": phone.id, "online": true }), now);
            let config = core.config.clone().unwrap();
            let uploads = Uploads::with_tools(&home, tools);
            Self { home, core, uploads, phone, config, now, next: 0 }
        }

        /// The phone sends `rpc`; what came back to it, opened, if anything.
        fn ask(&mut self, squad: Option<&str>, rpc: Value) -> Option<Value> {
            let frame = self.phone.send(&self.config, squad, rpc, self.now);
            assert!(frame.to_string().len() < super::super::core::RELAY_FRAME_MAX, "a piece fits in a relay frame");
            let mut answer = None;
            let mut actions = self.core.on_relay(&frame, self.now);
            while let Some(action) = actions.pop() {
                match action {
                    Action::Upload { device, id, piece } => {
                        let message = match self.uploads.piece(&device, *piece, self.now) {
                            Ok(result) => Message::response(id, result),
                            Err(refusal) => refusal.answer(id),
                        };
                        actions.extend(self.core.answer(&device, &message));
                    }
                    Action::UploadCancel { device, upload_id } => self.uploads.cancel(&device, &upload_id),
                    Action::Relay(_) => answer = Some(self.phone.open(&self.config, &action)["rpc"].clone()),
                    other => panic!("unexpected {other:?}"),
                }
            }
            answer
        }

        fn piece(&mut self, params: Value) -> Value {
            self.next += 1;
            self.ask(None, json!({ "jsonrpc": "2.0", "id": self.next, "method": "upload", "params": params })).expect("an answer")
        }

        /// Send `bytes` whole, a piece at a time; the last answer.
        fn send(&mut self, id: &str, name: &str, kind: &str, bytes: &[u8], first: Value) -> Value {
            let mut offset = 0;
            loop {
                let end = (offset + CHUNK_MAX).min(bytes.len());
                let mut params = json!({ "upload_id": id, "name": name, "kind": kind, "total": bytes.len(), "offset": offset, "data": b64(&bytes[offset..end]) });
                if offset == 0 {
                    for (k, v) in first.as_object().into_iter().flatten() {
                        params[k] = v.clone();
                    }
                }
                let answer = self.piece(params);
                assert!(answer["error"].is_null(), "{answer}");
                offset = end;
                if offset >= bytes.len() {
                    return answer["result"].clone();
                }
                assert_eq!(answer["result"], json!({ "received": offset }));
            }
        }

        /// The person's message from the phone, as the squad gets it.
        fn submit(&mut self, text: &str) -> String {
            let hello = json!({ "jsonrpc": "2.0", "id": 900, "method": "hello", "params": { "version": "app", "squad": "acme" } });
            let frame = self.phone.send(&self.config, Some("acme"), hello, self.now);
            assert!(matches!(&self.core.on_relay(&frame, self.now)[..], [Action::Open { .. }]));
            let submit = json!({ "jsonrpc": "2.0", "method": "command", "params": { "type": "submit", "squad": "acme", "session": 1, "text": text } });
            let frame = self.phone.send(&self.config, Some("acme"), submit, self.now);
            match &self.core.on_relay(&frame, self.now)[..] {
                [Action::Command { command: Command::Submit { text, .. }, .. }] => text.clone(),
                other => panic!("not a submit: {other:?}"),
            }
        }

        fn partials(&self) -> usize {
            std::fs::read_dir(partial_dir(&self.home)).map(|d| d.count()).unwrap_or(0)
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.home);
        }
    }

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    /// `ffmpeg` and `ffprobe` that only pretend: a 42-second 1920×1080 video,
    /// each frame a few bytes.
    fn fake_tools(home: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = home.with_extension("tools");
        std::fs::create_dir_all(&dir).unwrap();
        for (name, body) in [("ffprobe", "printf 'width=1920\\nheight=1080\\nduration=42.000000\\n'"), ("ffmpeg", "for last; do :; done; printf 'jpeg' > \"$last\"")] {
            let path = dir.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        dir
    }

    #[test]
    fn a_phone_learns_in_hello_that_its_files_are_taken() {
        let mut rig = Rig::new("hello", vec![]);
        let hello = json!({ "jsonrpc": "2.0", "id": 2, "method": "hello", "params": { "version": "app", "squad": "acme" } });
        let frame = rig.phone.send(&rig.config, Some("acme"), hello, rig.now);
        rig.core.on_relay(&frame, rig.now);
        let snapshot = equipe_core::protocol::Snapshot { squad: "acme".into(), ..Default::default() };
        let out = rig.core.opened(&rig.phone.id, "acme", &json!(2), Ok(equipe_core::protocol::rpc::Hello { version: "v".into(), snapshot }));
        let result = &rig.phone.open(&rig.config, &out[0])["rpc"]["result"];
        assert_eq!(result["uploads"], json!({ "chunk": 393216, "image": 8388608, "file": 26214400, "video": 67108864, "transcript": 65536 }));
        assert_eq!(result["snapshot"]["squad"], "acme");
    }

    #[test]
    fn a_photo_reaches_the_member_as_an_image_tag() {
        let mut rig = Rig::new("photo", vec![]);
        let photo: Vec<u8> = (0..612_345u32).map(|i| (i * 7) as u8).collect();
        let done = rig.send("photo-0001", "IMG_0421.JPG", "image", &photo, json!({}));
        let path = PathBuf::from(done["path"].as_str().unwrap());
        assert_eq!(done["received"], 612_345);
        assert_eq!(std::fs::read(&path).unwrap(), photo);
        assert_eq!(path.parent().unwrap(), gateway::uploads_dir(&rig.home));
        assert!(path.file_name().unwrap().to_str().unwrap().ends_with("-IMG_0421.jpg"));
        assert_eq!((mode(&path), mode(&gateway::uploads_dir(&rig.home))), (0o600, 0o700));
        assert_eq!(done["tag"], format!("[image: {}]", path.display()));
        assert_eq!(rig.partials(), 0, "nothing left waiting");
        // The phone appends the tag to what was typed; the squad gets it as the person's words.
        let text = rig.submit(&format!("이거 봐 줘 {}", done["tag"].as_str().unwrap()));
        let found = tags::find(&text);
        assert_eq!((found.len(), found[0].kind, found[0].path.as_str()), (1, TagKind::Image, path.to_str().unwrap()));
        assert_eq!(found[0].name(), "IMG_0421.jpg");
    }

    #[test]
    fn a_ten_megabyte_file_comes_in_pieces_and_is_kept_whole() {
        let mut rig = Rig::new("file", vec![]);
        let file: Vec<u8> = (0..10 * 1024 * 1024u32).map(|i| b'a' + (i % 26) as u8).collect();
        let done = rig.send("file-0001", "/private/var/mobile/report final.bin", "file", &file, json!({}));
        let path = PathBuf::from(done["path"].as_str().unwrap());
        assert!(path.to_str().unwrap().ends_with("-report-final.bin"), "no folder of the phone's: {}", path.display());
        assert_eq!(std::fs::read(&path).unwrap(), file);
        assert_eq!(done["tag"], format!("[file: {}]", path.display()), "too big to count lines");
        let text = rig.submit(done["tag"].as_str().unwrap());
        assert_eq!(tags::find(&text)[0].kind, TagKind::File);
        // A small text file gets its lines.
        let done = rig.send("notes-001", "notes.md", "file", "one\ntwo\nthree\n".as_bytes(), json!({}));
        assert_eq!(done["tag"], format!("[file: {} · 3 lines]", done["path"].as_str().unwrap()));
        let done = rig.send("spec-0001", "spec.pdf", "pdf", b"%PDF-1.7", json!({ "pages": 12 }));
        assert_eq!(tags::find(done["tag"].as_str().unwrap())[0].pages, Some(12));
    }

    #[test]
    fn a_sound_is_kept_with_what_the_phone_heard_beside_it() {
        let mut rig = Rig::new("audio", vec![]);
        let sound = vec![7u8; 50_000];
        let done = rig.send("voice-001", "Voice memo.m4a", "file", &sound, json!({ "transcript": "내일 회의는 3시로 옮겨 주세요.\n자료는 금요일까지.", "duration": 31.4 }));
        let found = tags::find(done["tag"].as_str().unwrap());
        assert_eq!(found.len(), 2, "{}", done["tag"]);
        assert_eq!((found[0].kind, found[0].path.as_str(), found[0].duration.as_deref()), (TagKind::Audio, done["path"].as_str().unwrap(), Some("0:31")));
        assert_eq!(std::fs::read(&found[0].path).unwrap(), sound, "the sound itself is kept");
        assert_eq!((found[1].kind, found[1].lines), (TagKind::File, Some(2)));
        assert!(found[1].path.ends_with("-Voice-memo-transcript.txt"), "{}", found[1].path);
        assert_eq!(std::fs::read_to_string(&found[1].path).unwrap(), "내일 회의는 3시로 옮겨 주세요.\n자료는 금요일까지.\n");
        let text = rig.submit(&format!("음성 메모 {}", done["tag"].as_str().unwrap()));
        assert_eq!(tags::find(&text).len(), 2);
        // Without a transcript: the sound alone.
        let done = rig.send("voice-002", "a.wav", "file", &sound, json!({}));
        assert_eq!(done["tag"], format!("[audio: {}]", done["path"].as_str().unwrap()));
        // A transcript means nothing on a picture.
        let done = rig.send("pic-00001", "a.png", "image", &sound, json!({ "transcript": "hi" }));
        assert_eq!(tags::find(done["tag"].as_str().unwrap()).len(), 1);
    }

    #[test]
    fn a_video_becomes_frames_and_a_transcript() {
        let home = std::env::temp_dir().join(format!("equipe-phone-uploads-video-{}", std::process::id()));
        let tools = fake_tools(&home);
        let mut rig = Rig::new("video", vec![tools.clone()]);
        let video = vec![1u8; 900_000];
        let done = rig.send("clip-0001", "IMG_0500.MOV", "video", &video, json!({ "transcript": "여기 버튼이 안 눌려요" }));
        assert!(done["notice"].is_null());
        let found = tags::find(done["tag"].as_str().unwrap());
        let tag = &found[0];
        assert_eq!((tag.kind, tag.path.as_str(), tag.duration.as_deref(), tag.dims), (TagKind::Video, done["path"].as_str().unwrap(), Some("0:42"), Some((1920, 1080))));
        assert_eq!(tag.frames.len(), 4, "42 s: four frames");
        assert_eq!(tag.frames.iter().map(|(_, at)| at.as_str()).collect::<Vec<_>>(), ["0:05", "0:16", "0:26", "0:37"]);
        for (frame, _) in &tag.frames {
            assert!(frame.ends_with(".jpg") && Path::new(frame).parent() == Some(gateway::uploads_dir(&rig.home).as_path()), "{frame}");
            assert_eq!(std::fs::read(frame).unwrap(), b"jpeg");
        }
        assert_eq!((found[1].kind, found[1].lines), (TagKind::File, Some(1)));
        let text = rig.submit(done["tag"].as_str().unwrap());
        assert_eq!(tags::find(&text)[0].frames.len(), 4);
        let _ = std::fs::remove_dir_all(tools);
    }

    #[test]
    fn a_video_without_ffmpeg_is_kept_and_the_phone_is_told() {
        let mut rig = Rig::new("noffmpeg", vec![]);
        let done = rig.send("clip-0002", "clip.mp4", "video", &[1u8; 1000], json!({ "duration": 12.0, "transcript": "안녕" }));
        assert_eq!(done["notice"], "no_ffmpeg");
        let found = tags::find(done["tag"].as_str().unwrap());
        assert_eq!((found[0].kind, found[0].frames.len(), found[0].duration.as_deref()), (TagKind::Video, 0, Some("0:12")));
        assert_eq!(found[1].kind, TagKind::File, "the transcript still goes");
    }

    /// With the real `ffmpeg`, when this machine has it: a two-second clip.
    #[test]
    fn a_real_short_video_gives_real_frames() {
        let Some(ffmpeg) = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].iter().map(|d| Path::new(d).join("ffmpeg")).find(|p| p.is_file()) else {
            return;
        };
        let mut rig = Rig::new("realvideo", ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].iter().map(PathBuf::from).collect());
        let clip = rig.home.join("clip.mp4");
        let made = std::process::Command::new(ffmpeg)
            .args(["-v", "error", "-f", "lavfi", "-i", "testsrc=duration=2:size=640x360:rate=10", "-pix_fmt", "yuv420p"])
            .arg(&clip)
            .status()
            .unwrap();
        if !made.success() {
            return; // an ffmpeg without the encoder
        }
        let bytes = std::fs::read(&clip).unwrap();
        let done = rig.send("real-0001", "clip.mp4", "video", &bytes, json!({}));
        let tag = &tags::find(done["tag"].as_str().unwrap())[0];
        assert_eq!((tag.dims, tag.frames.len()), (Some((640, 360)), 4), "{}", done["tag"]);
        for (frame, _) in &tag.frames {
            assert_eq!(&std::fs::read(frame).unwrap()[..2], [0xff, 0xd8], "a JPEG");
        }
    }

    #[test]
    fn what_is_too_large_is_refused_before_anything_is_kept() {
        let mut rig = Rig::new("large", vec![]);
        for (kind, name, total, limit) in [("image", "a.jpg", IMAGE_MAX + 1, IMAGE_MAX), ("file", "a.zip", FILE_MAX + 1, FILE_MAX), ("pdf", "a.pdf", FILE_MAX + 1, FILE_MAX), ("video", "a.mov", VIDEO_MAX + 1, VIDEO_MAX)] {
            let answer = rig.piece(json!({ "upload_id": "big-00001", "name": name, "kind": kind, "total": total, "offset": 0, "data": b64(b"abc") }));
            assert_eq!(answer["error"], json!({ "code": -32602, "message": "too large", "data": { "limit": limit } }), "{kind}");
        }
        // A piece over the limit, or past the total.
        let answer = rig.piece(json!({ "upload_id": "big-00002", "name": "a.zip", "kind": "file", "total": 1 << 20, "offset": 0, "data": b64(&vec![0u8; CHUNK_MAX + 1]) }));
        assert_eq!(answer["error"]["message"], "piece too large");
        let answer = rig.piece(json!({ "upload_id": "big-00003", "name": "a.zip", "kind": "file", "total": 2, "offset": 0, "data": b64(b"abc") }));
        assert_eq!(answer["error"]["message"], "piece too large");
        let answer = rig.piece(json!({ "upload_id": "big-00004", "name": "a.m4a", "kind": "file", "total": 3, "offset": 0, "data": b64(b"abc"), "transcript": "a".repeat(TRANSCRIPT_MAX + 1) }));
        assert_eq!(answer["error"]["data"]["limit"], TRANSCRIPT_MAX);
        // The kind decides the extensions; a name, never a folder.
        let answer = rig.piece(json!({ "upload_id": "big-00005", "name": "evil.sh", "kind": "image", "total": 3, "offset": 0, "data": b64(b"abc") }));
        assert_eq!(answer["error"]["code"], -32602);
        let answer = rig.piece(json!({ "upload_id": "../../etc", "name": "a.png", "kind": "image", "total": 3, "offset": 0, "data": b64(b"abc") }));
        assert_eq!(answer["error"]["code"], -32602);
        let answer = rig.piece(json!({ "upload_id": "big-00006", "name": "a.png", "kind": "image", "total": 3, "offset": 0, "data": "!!!" }));
        assert_eq!(answer["error"]["message"], "data isn't base64");
        let answer = rig.piece(json!({ "upload_id": "big-00007", "name": "a.png" }));
        assert_eq!(answer["error"]["code"], -32602);
        assert_eq!(std::fs::read_dir(gateway::uploads_dir(&rig.home)).map(|d| d.filter(|e| e.as_ref().unwrap().file_type().unwrap().is_file()).count()).unwrap_or(0), 0, "nothing kept");
        assert_eq!(rig.partials(), 0);
    }

    #[test]
    fn a_connection_lost_halfway_resumes_where_the_daemon_is() {
        let mut rig = Rig::new("resume", vec![]);
        let file: Vec<u8> = (0..(CHUNK_MAX * 3 + 10) as u32).map(|i| i as u8).collect();
        let total = file.len();
        let piece = |offset: usize, len: usize| json!({ "upload_id": "resume-01", "name": "a.bin", "kind": "file", "total": total, "offset": offset, "data": b64(&file[offset..offset + len]) });
        assert_eq!(rig.piece(piece(0, CHUNK_MAX))["result"], json!({ "received": CHUNK_MAX }));
        assert_eq!(rig.piece(piece(CHUNK_MAX, CHUNK_MAX))["result"], json!({ "received": 2 * CHUNK_MAX }));
        // The phone's socket drops; the answer to the next piece was lost on the way.
        assert!(rig.core.on_relay(&json!({ "type": "presence", "device": rig.phone.id, "online": false }), rig.now).contains(&Action::Close { device: rig.phone.id.clone() }));
        rig.phone.reconnect();
        rig.core.on_relay(&json!({ "type": "presence", "device": rig.phone.id, "online": true }), rig.now);
        // On the new connection it sends the second piece again: had already, nothing changes.
        assert_eq!(rig.piece(piece(CHUNK_MAX, CHUNK_MAX))["result"], json!({ "received": 2 * CHUNK_MAX }));
        // Skipping ahead is refused with where to go on from.
        let skipped = rig.piece(piece(3 * CHUNK_MAX, 10));
        assert_eq!(skipped["error"], json!({ "code": -32602, "message": "send from received", "data": { "received": 2 * CHUNK_MAX } }));
        // The same id with another name or size is another file.
        let mut other = piece(2 * CHUNK_MAX, CHUNK_MAX);
        other["total"] = json!(total + 1);
        assert_eq!(rig.piece(other)["error"]["message"], "not the same upload");
        assert_eq!(rig.piece(piece(2 * CHUNK_MAX, CHUNK_MAX))["result"], json!({ "received": 3 * CHUNK_MAX }));
        let done = rig.piece(piece(3 * CHUNK_MAX, 10))["result"].clone();
        assert_eq!(std::fs::read(done["path"].as_str().unwrap()).unwrap(), file);
        // The last piece again (its answer lost): the same answer, the file kept once.
        assert_eq!(rig.piece(piece(3 * CHUNK_MAX, 10))["result"], done);
        assert_eq!(std::fs::read_dir(gateway::uploads_dir(&rig.home)).unwrap().filter(|e| e.as_ref().unwrap().file_type().unwrap().is_file()).count(), 1);
    }

    #[test]
    fn a_file_the_daemon_forgot_starts_over_and_one_cancelled_or_left_goes() {
        let mut rig = Rig::new("forget", vec![]);
        let piece = |id: &str, offset: usize| json!({ "upload_id": id, "name": "a.bin", "kind": "file", "total": 2 * CHUNK_MAX, "offset": offset, "data": b64(&vec![1u8; CHUNK_MAX]) });
        // A piece past 0 for an id the daemon doesn't know (it restarted): from the start.
        assert_eq!(rig.piece(piece("unknown-1", CHUNK_MAX))["error"]["data"], json!({ "received": 0 }));
        // Cancelled: what came is thrown away; the notification has no answer.
        rig.piece(piece("cancel-01", 0));
        assert_eq!(rig.partials(), 1);
        assert!(rig.ask(None, json!({ "jsonrpc": "2.0", "method": "upload_cancel", "params": { "upload_id": "cancel-01" } })).is_none());
        assert_eq!(rig.partials(), 0);
        assert_eq!(rig.piece(piece("cancel-01", CHUNK_MAX))["error"]["data"], json!({ "received": 0 }));
        // At most four at once from one phone.
        for n in 0..4 {
            assert!(rig.piece(piece(&format!("many-000{n}"), 0))["error"].is_null());
        }
        assert_eq!(rig.piece(piece("many-0004", 0))["error"], json!({ "code": -32000, "message": "too many uploads at once" }));
        // An hour without a piece: thrown away.
        rig.uploads.sweep(rig.now + IDLE_MS);
        assert_eq!(rig.partials(), 0);
        // A daemon starting again forgets what was waiting.
        rig.piece(piece("restart-1", 0));
        let _ = Uploads::with_tools(&rig.home, vec![]);
        assert_eq!(rig.partials(), 0);
    }

    #[test]
    fn upload_is_all_a_phone_gains() {
        let mut rig = Rig::new("reach", vec![]);
        // read_upload stays a screen's.
        let answer = rig.ask(None, json!({ "jsonrpc": "2.0", "id": 5, "method": "read_upload", "params": { "path": "/etc/hosts" } })).unwrap();
        assert_eq!(answer["error"]["code"], -32601);
        // An upload that doesn't parse is refused, not dropped.
        let answer = rig.ask(None, json!({ "jsonrpc": "2.0", "id": 6, "method": "upload", "params": { "name": "a.png", "data": "AA==" } })).unwrap();
        assert_eq!(answer["error"]["code"], -32602);
    }

    #[test]
    fn frames_are_taken_as_the_desktop_takes_them() {
        assert_eq!(frame_times(42.).len(), 4);
        assert_eq!(frame_times(300.).len(), 12);
        assert_eq!(frame_times(80.), (0..8).map(|i| 80. * (i as f64 + 0.5) / 8.).collect::<Vec<_>>());
        assert_eq!((clock(31.4), clock(3723.)), ("0:31".into(), "1:02:03".into()));
    }
}
