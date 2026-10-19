//! What is attached to a message: pictures, videos, PDFs, text and code,
//! documents, sounds — any file. What each kind may be, how one is made ready
//! (all off the UI thread: a picture shrunk, a video's frames taken, a PDF's
//! pages counted), how it reaches the daemon's machine, the strip of what is
//! attached so far, and how the message that carries them reads.
//!
//! Like the terminal console, an attachment reaches a member as a file on the
//! daemon's machine and a tag in the message — `[image: p]`, `[pdf: p · 12
//! pages]`, `[video: p · 1:42 · 1920×1080 · sheet: s · frames: f@0:03, …]`,
//! `[audio: p · 0:31]`, `[file: p]` — the protocol has no attachment field.
//! A daemon on this Mac gets a clone of the file in its `uploads` folder (no
//! copy, no line on the socket); one on another machine gets small files as an
//! `upload` line and big ones over an SSH channel of their own.

use equipe_core::t;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use equipe_core::protocol::rpc::UploadKind;
use gpui_kit::{Image, ImageFormat};
use image::ImageFormat as Format;
use serde::{Deserialize, Serialize};

use crate::media;

/// How many attachments one message may carry.
pub const MAX_ATTACHMENTS: usize = 20;
/// The largest one picture may be once ready, in bytes.
pub const MAX_BYTES: usize = 8 << 20;
/// A longer side than this is shrunk before it is kept (what a member can use).
pub const LONG_SIDE: u32 = 1568;
/// The thumbnail's side in pixels: 72 points at 2x.
pub const THUMB: u32 = 144;
/// A picture bigger than this isn't even read.
const READ_MAX: u64 = 64 << 20;
/// The largest file of any other kind.
pub const FILE_MAX: u64 = equipe_core::gateway::RECEIVE_MAX;
/// To a daemon on another machine, files up to this go as an `upload` line;
/// bigger ones over an SSH channel of their own.
pub const LINE_MAX: u64 = 8 << 20;
/// A paste longer than this becomes an attachment of its own (⌘⇧V pastes it inline).
pub const PASTE_CHARS: usize = 2000;
pub const PASTE_LINES: usize = 40;

// ── kinds ──

/// What an attachment is, which decides how it is shown and what a member gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Image,
    Video,
    Audio,
    Pdf,
    /// Text and code: read as it is.
    Text,
    /// Word, Excel, Keynote …: sent as a file.
    Doc,
    File,
}

const IMAGES: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "tiff", "tif", "heic", "heif", "bmp"];
const VIDEOS: &[&str] = &["mov", "mp4", "m4v", "webm", "mkv", "avi", "mpg", "mpeg", "3gp"];
const AUDIOS: &[&str] = &["m4a", "mp3", "wav", "aiff", "aif", "flac", "aac", "ogg", "opus", "caf"];
const DOCS: &[&str] = &["docx", "doc", "xlsx", "xls", "pptx", "ppt", "pages", "numbers", "key", "rtf", "odt", "ods", "odp", "epub"];
const TEXTS: &[&str] = &[
    "md", "markdown", "txt", "text", "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "json", "jsonc", "jsonl", "csv", "tsv", "log", "toml", "yaml",
    "yml", "html", "htm", "css", "scss", "sass", "less", "sh", "zsh", "bash", "fish", "swift", "go", "java", "kt", "kts", "c", "h", "cc", "cpp", "hpp",
    "m", "mm", "rb", "php", "sql", "xml", "ini", "cfg", "conf", "env", "lock", "gradle", "vue", "svelte", "astro", "lua", "dart", "ex", "exs", "erl",
    "hs", "ml", "scala", "r", "jl", "proto", "graphql", "gql", "diff", "patch", "srt", "vtt", "tex", "rst", "org", "plist", "svg",
];
const TEXT_NAMES: &[&str] = &["makefile", "dockerfile", "justfile", "procfile", "gemfile", "rakefile", "license", "readme"];

fn extension(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

impl Kind {
    /// By the file's name.
    pub fn of(path: &Path) -> Self {
        let ext = extension(path);
        let has = |list: &[&str]| list.contains(&ext.as_str());
        if has(IMAGES) {
            Self::Image
        } else if has(VIDEOS) {
            Self::Video
        } else if has(AUDIOS) {
            Self::Audio
        } else if ext == "pdf" {
            Self::Pdf
        } else if has(DOCS) {
            Self::Doc
        } else if has(TEXTS) || (ext.is_empty() && path.file_name().and_then(|n| n.to_str()).is_some_and(|n| TEXT_NAMES.contains(&n.to_ascii_lowercase().as_str()))) {
            Self::Text
        } else {
            Self::File
        }
    }

    /// How it is uploaded. A picture says nothing, as before attachments: any
    /// daemon takes it.
    pub fn upload(self) -> Option<UploadKind> {
        match self {
            Self::Image => None,
            Self::Video => Some(UploadKind::Video),
            Self::Pdf => Some(UploadKind::Pdf),
            _ => Some(UploadKind::File),
        }
    }

    /// What `equipe receive` is told it is.
    fn word(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Pdf => "pdf",
            _ => "file",
        }
    }

    /// The largest file of this kind.
    pub fn limit(self) -> u64 {
        if self == Self::Image { READ_MAX } else { FILE_MAX }
    }
}

/// The short word on a chip: the extension (`PDF`, `RS`, `DOCX`), or the kind's.
pub fn badge(name: &str, kind: Kind) -> String {
    let ext = extension(Path::new(name)).to_ascii_uppercase();
    match (ext.len(), kind) {
        (1..=4, _) => ext,
        (_, Kind::Text) => "TXT".into(),
        _ => "FILE".into(),
    }
}

/// `1.2 MB`, `340 KB`.
pub fn size_label(bytes: u64) -> String {
    if bytes >= 1 << 30 {
        format!("{:.1} GB", bytes as f64 / f64::from(1 << 30))
    } else if bytes >= 1 << 20 {
        format!("{:.1} MB", bytes as f64 / f64::from(1 << 20))
    } else {
        format!("{} KB", bytes.div_ceil(1024).max(1))
    }
}

fn limit_label(bytes: u64) -> String {
    if bytes >= 1 << 30 { format!("{} GB", bytes >> 30) } else { format!("{} MB", bytes >> 20) }
}

/// Before a drop or a paste: is this a file we take? Its kind and size, or why not.
pub fn check(path: &Path) -> Result<(Kind, u64), String> {
    let meta = std::fs::metadata(path).map_err(|_| t!("attach.unreadable_file").to_owned())?;
    if meta.is_dir() {
        return Err(t!("attach.folder").to_owned());
    }
    let kind = Kind::of(path);
    if meta.len() > kind.limit() {
        return Err(t!("attach.too_big_limit", limit = limit_label(kind.limit())));
    }
    if meta.len() == 0 {
        return Err(t!("attach.empty").to_owned());
    }
    Ok((kind, meta.len()))
}

pub(crate) fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| "file".into(), |n| n.to_string_lossy().into_owned())
}

// ── where an attachment comes from ──

#[derive(Debug, Clone)]
pub enum Source {
    /// The clipboard, a drop from another app, the inbox: a picture's bytes.
    Bytes { name: String, bytes: Vec<u8> },
    /// A file dragged in, picked, or copied in Finder.
    File(PathBuf),
    /// A long paste, kept as a text file.
    Text { name: String, text: String },
}

impl Source {
    pub fn name(&self) -> String {
        match self {
            Self::Bytes { name, .. } | Self::Text { name, .. } => name.clone(),
            Self::File(path) => file_name(path),
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            Self::Bytes { .. } => Kind::Image,
            Self::Text { .. } => Kind::Text,
            Self::File(path) => Kind::of(path),
        }
    }

    /// Its size, as far as known before it is read.
    pub fn size(&self) -> u64 {
        match self {
            Self::Bytes { bytes, .. } => bytes.len() as u64,
            Self::Text { text, .. } => text.len() as u64,
            Self::File(path) => std::fs::metadata(path).map_or(0, |m| m.len()),
        }
    }

    /// The file on this Mac it was, to attach again later.
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::File(path) => Some(path),
            _ => None,
        }
    }
}

// ── made ready ──

/// What is known of an attachment, for its chip and its tag.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dims: Option<(u32, u32)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<f32>,
    /// How many of a video's frames go with it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub frames: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// Bytes or a file, to be kept on the daemon's machine under `name`.
#[derive(Debug, Clone)]
pub enum Payload {
    Bytes { name: String, bytes: Vec<u8> },
    File { name: String, path: PathBuf },
}

impl Payload {
    fn name(&self) -> &str {
        match self {
            Self::Bytes { name, .. } | Self::File { name, .. } => name,
        }
    }

    fn size(&self) -> u64 {
        match self {
            Self::Bytes { bytes, .. } => bytes.len() as u64,
            Self::File { path, .. } => std::fs::metadata(path).map_or(0, |m| m.len()),
        }
    }
}

/// An attachment made ready to send and show.
#[derive(Debug)]
pub struct Prepared {
    pub kind: Kind,
    /// What its chip says.
    pub name: String,
    pub size: u64,
    /// A small PNG for the chip: the picture, a video's first frame, a PDF's first page.
    pub thumb: Option<Vec<u8>>,
    pub meta: Meta,
    pub main: Payload,
    /// A video's frames, at their times, and the sheet of them all.
    pub frames: Vec<(f64, Payload)>,
    pub sheet: Option<Payload>,
}

fn stem(name: &str) -> String {
    Path::new(name).file_stem().map_or_else(|| "file".into(), |s| s.to_string_lossy().into_owned())
}

fn thumb_of(image: &image::RgbImage) -> Option<Vec<u8>> {
    let small = image::DynamicImage::ImageRgb8(image.clone()).thumbnail(THUMB, THUMB).to_rgb8();
    media::png(&small).ok()
}

fn png(image: &image::DynamicImage) -> Result<Vec<u8>, String> {
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, Format::Png).map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

fn image_extension(kind: Format) -> Option<&'static str> {
    match kind {
        Format::Png => Some("png"),
        Format::Jpeg => Some("jpg"),
        Format::Gif => Some("gif"),
        Format::WebP => Some("webp"),
        _ => None,
    }
}

/// Make an attachment ready: read what its chip and its tag need. Blocking;
/// what stays in memory is the bytes to send, never a decoded picture.
pub fn prepare(source: Source) -> Result<Prepared, String> {
    let name = source.name();
    match source {
        Source::Bytes { bytes, .. } => picture(name, bytes),
        Source::Text { text, .. } => {
            let lines = text.lines().count();
            let size = text.len() as u64;
            Ok(Prepared { kind: Kind::Text, name: name.clone(), size, thumb: None, meta: Meta { lines: Some(lines), ..Meta::default() }, main: Payload::Bytes { name, bytes: text.into_bytes() }, frames: Vec::new(), sheet: None })
        }
        Source::File(path) => {
            let (kind, size) = check(&path)?;
            let file = |meta: Meta, thumb: Option<Vec<u8>>| Prepared { kind, name: name.clone(), size, thumb, meta, main: Payload::File { name: name.clone(), path: path.clone() }, frames: Vec::new(), sheet: None };
            match kind {
                Kind::Image => {
                    let bytes = std::fs::read(&path).map_err(|_| t!("attach.unreadable_file").to_owned())?;
                    picture(name.clone(), bytes).or_else(|why| converted(&path, &name).ok_or(why))
                }
                Kind::Video => Ok(video(&path, name.clone(), size).unwrap_or_else(|_| file(Meta::default(), media::quick_look(&path, THUMB).as_ref().and_then(thumb_of)))),
                Kind::Pdf => {
                    let (pages, first) = media::pdf(&path, THUMB * 2).map_err(|_| t!("attach.pdf_unreadable").to_owned())?;
                    Ok(file(Meta { pages: Some(pages), ..Meta::default() }, first.as_ref().and_then(thumb_of)))
                }
                Kind::Text => Ok(file(Meta { lines: count_lines(&path), ..Meta::default() }, None)),
                Kind::Audio => Ok(file(Meta { duration: media::audio_duration(&path), ..Meta::default() }, None)),
                Kind::Doc | Kind::File => Ok(file(Meta::default(), media::quick_look(&path, THUMB).as_ref().and_then(thumb_of))),
            }
        }
    }
}

/// Decode, shrink what is too long, thumbnail.
fn picture(name: String, bytes: Vec<u8>) -> Result<Prepared, String> {
    let kind = image::guess_format(&bytes).map_err(|_| t!("attach.unreadable").to_owned())?;
    let decoded = image::load_from_memory_with_format(&bytes, kind).map_err(|_| t!("attach.unreadable").to_owned())?;
    let thumb = png(&decoded.thumbnail(THUMB, THUMB))?;
    let dims = (decoded.width(), decoded.height());
    let shrink = decoded.width().max(decoded.height()) > LONG_SIDE;
    let (kept, bytes) = match (shrink, image_extension(kind)) {
        // Too long, or a format the daemon doesn't take (TIFF from Preview): as PNG.
        (true, _) | (false, None) => {
            let kept = if shrink { decoded.resize(LONG_SIDE, LONG_SIDE, image::imageops::FilterType::Triangle) } else { decoded };
            (format!("{}.png", stem(&name)), png(&kept)?)
        }
        (false, Some(ext)) => (format!("{}.{ext}", stem(&name)), bytes),
    };
    if bytes.len() > MAX_BYTES {
        return Err(t!("attach.too_big", mb = MAX_BYTES >> 20));
    }
    let size = bytes.len() as u64;
    Ok(Prepared { kind: Kind::Image, name: kept.clone(), size, thumb: Some(thumb), meta: Meta { dims: Some(dims), ..Meta::default() }, main: Payload::Bytes { name: kept, bytes }, frames: Vec::new(), sheet: None })
}

/// A picture the decoder doesn't know (HEIC from an iPhone): macOS turns it into a JPEG.
fn converted(path: &Path, name: &str) -> Option<Prepared> {
    let out = std::env::temp_dir().join(format!("equipe-sips-{}-{}.jpg", std::process::id(), stem(name)));
    let ok = std::process::Command::new("/usr/bin/sips").args(["-s", "format", "jpeg"]).arg(path).arg("--out").arg(&out).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success());
    let bytes = ok.then(|| std::fs::read(&out).ok()).flatten();
    let _ = std::fs::remove_file(&out);
    picture(format!("{}.jpg", stem(name)), bytes?).ok()
}

/// A video's frames and the sheet of them; its first frame is its chip's picture.
fn video(path: &Path, name: String, size: u64) -> Result<Prepared, String> {
    let video = media::video(path)?;
    let base = stem(&name);
    let mut frames = Vec::new();
    for (i, frame) in video.frames.iter().enumerate() {
        frames.push((frame.at, Payload::Bytes { name: format!("{base}-frame{:02}.jpg", i + 1), bytes: media::jpeg(&frame.image)? }));
    }
    let sheet = Payload::Bytes { name: format!("{base}-sheet.jpg"), bytes: media::jpeg(&media::contact_sheet(&video.frames))? };
    let meta = Meta { duration: Some(video.duration), dims: Some((video.width, video.height)), fps: video.fps, frames: frames.len(), ..Meta::default() };
    Ok(Prepared { kind: Kind::Video, thumb: video.frames.first().and_then(|f| thumb_of(&f.image)), name: name.clone(), size, meta, main: Payload::File { name, path: path.to_owned() }, frames, sheet: Some(sheet) })
}

/// How many lines a text file has; `None` for one that isn't text after all.
fn count_lines(path: &Path) -> Option<usize> {
    use std::io::Read as _;
    let mut head = Vec::new();
    std::fs::File::open(path).ok()?.take(32 << 20).read_to_end(&mut head).ok()?;
    if head.iter().take(8192).any(|b| *b == 0) {
        return None;
    }
    let newlines = head.iter().filter(|b| **b == b'\n').count();
    Some(newlines + usize::from(!head.ends_with(b"\n") && !head.is_empty()))
}

// ── to the daemon's machine ──

/// Where attachments are kept: this Mac's daemon's folder, or another machine's.
#[derive(Clone)]
pub enum Store {
    /// The daemon on this Mac: files are cloned into its `uploads` folder.
    Local { home: PathBuf },
    /// Another machine: an `upload` line, or `equipe receive` over SSH for big files.
    Remote { uploader: crate::daemon::Uploader, ssh: Option<(crate::ssh::Launcher, crate::target::SshHost)> },
}

/// One attachment kept on the daemon's machine: its path there and its tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ready {
    pub path: String,
    pub tag: String,
}

impl Store {
    /// Keep `payload` there; its path. `progress` hears the share sent (0–1).
    pub fn put(&self, payload: &Payload, kind: Kind, progress: &dyn Fn(f32)) -> Result<String, String> {
        match self {
            Self::Local { home } => put_local(home, payload, kind),
            Self::Remote { uploader, ssh } => {
                let size = payload.size();
                match payload {
                    Payload::Bytes { name, bytes } => uploader.upload(name, bytes, kind.upload()).map(|p| p.to_string_lossy().into_owned()),
                    Payload::File { name, path } if size <= LINE_MAX => {
                        let bytes = std::fs::read(path).map_err(|_| t!("attach.unreadable_file").to_owned())?;
                        uploader.upload(name, &bytes, kind.upload()).map(|p| p.to_string_lossy().into_owned())
                    }
                    Payload::File { name, path } => {
                        let Some((launcher, host)) = ssh else {
                            return Err(t!("attach.remote_too_big", mb = LINE_MAX >> 20));
                        };
                        let (stem, ext) = equipe_core::gateway::upload_name(name, kind.upload())?;
                        let tidy = if ext.is_empty() { stem } else { format!("{stem}.{ext}") };
                        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
                        let sent = |n: u64| progress(n as f32 / size.max(1) as f32);
                        runtime.block_on(launcher.send_file(host, path, &tidy, kind.word(), sent)).map_err(|failure| match failure {
                            crate::failure::Failure::Other(why) => why,
                            _ => t!("attach.remote_old").to_owned(),
                        })
                    }
                }
            }
        }
    }
}

/// Into this Mac's daemon's `uploads`: bytes written, a file cloned (APFS:
/// no copy is made) or else copied — 0600, and dated now so the daemon's
/// week-old sweep doesn't take it at once.
fn put_local(home: &Path, payload: &Payload, kind: Kind) -> Result<String, String> {
    use std::io::Write as _;
    let (mut file, path) = equipe_core::gateway::create_upload(home, payload.name(), kind.upload())?;
    match payload {
        Payload::Bytes { bytes, .. } => file.write_all(bytes).map_err(|e| e.to_string())?,
        Payload::File { path: from, .. } => {
            drop(file);
            let _ = std::fs::remove_file(&path);
            if !clone(from, &path) {
                std::fs::copy(from, &path).map_err(|_| t!("attach.unreadable_file").to_owned())?;
            }
            use std::os::unix::fs::PermissionsExt as _;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            let _ = std::fs::File::options().write(true).open(&path).and_then(|f| f.set_modified(std::time::SystemTime::now()));
        }
    }
    Ok(path.to_string_lossy().into_owned())
}

#[cfg(target_os = "macos")]
fn clone(from: &Path, to: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt as _;
    let (Ok(from), Ok(to)) = (std::ffi::CString::new(from.as_os_str().as_bytes()), std::ffi::CString::new(to.as_os_str().as_bytes())) else {
        return false;
    };
    // SAFETY: two valid NUL-terminated paths; clonefile reads them only.
    unsafe { libc::clonefile(from.as_ptr(), to.as_ptr(), 0) == 0 }
}

#[cfg(not(target_os = "macos"))]
fn clone(_: &Path, _: &Path) -> bool {
    false
}

/// Send what `prepared` needs on the daemon's machine; its tag.
pub fn deliver(prepared: &Prepared, store: &Store, progress: &dyn Fn(f32)) -> Result<Ready, String> {
    let parts = 1 + prepared.frames.len() + usize::from(prepared.sheet.is_some());
    let share = |done: usize| move |f: f32| progress((done as f32 + f.clamp(0., 1.)) / parts as f32);
    let path = store.put(&prepared.main, prepared.kind, &share(0))?;
    let mut frames = Vec::new();
    for (i, (at, frame)) in prepared.frames.iter().enumerate() {
        frames.push((*at, store.put(frame, Kind::Image, &share(1 + i))?));
    }
    let sheet = match &prepared.sheet {
        Some(sheet) => Some(store.put(sheet, Kind::Image, &share(parts - 1))?),
        None => None,
    };
    progress(1.);
    Ok(Ready { tag: tag(prepared.kind, &path, &prepared.meta, &frames, sheet.as_deref()), path })
}

/// How an attachment reads in the message (and to a member: see `roles.rs`).
pub fn tag(kind: Kind, path: &str, meta: &Meta, frames: &[(f64, String)], sheet: Option<&str>) -> String {
    let mut parts = vec![path.to_owned()];
    let word = match kind {
        Kind::Image => "image",
        Kind::Video => "video",
        Kind::Audio => "audio",
        Kind::Pdf => "pdf",
        Kind::Text | Kind::Doc | Kind::File => "file",
    };
    if let Some(pages) = meta.pages {
        parts.push(if pages == 1 { "1 page".into() } else { format!("{pages} pages") });
    }
    if let Some(lines) = meta.lines.filter(|_| kind == Kind::Text) {
        parts.push(if lines == 1 { "1 line".into() } else { format!("{lines} lines") });
    }
    if let Some(duration) = meta.duration.filter(|_| matches!(kind, Kind::Video | Kind::Audio)) {
        parts.push(media::clock(duration));
    }
    if let Some((w, h)) = meta.dims.filter(|_| kind == Kind::Video) {
        parts.push(format!("{w}×{h}"));
    }
    if let Some(sheet) = sheet {
        parts.push(format!("sheet: {sheet}"));
    }
    if !frames.is_empty() {
        let list: Vec<String> = frames.iter().map(|(at, p)| format!("{p}@{}", media::clock(*at))).collect();
        parts.push(format!("frames: {}", list.join(", ")));
    }
    format!("[{word}: {}]", parts.join(" · "))
}

// ── the strip ──

/// Where one attachment stands.
#[derive(Debug, Clone, PartialEq)]
pub enum Stage {
    /// Being read on this Mac: frames taken, pages counted.
    Preparing,
    /// On its way to the daemon's machine; how far (0–1).
    Sending(f32),
    Ready(Ready),
    /// It couldn't be attached, and why: shown until dismissed.
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct Item {
    pub id: u64,
    pub kind: Kind,
    pub name: String,
    pub size: u64,
    pub thumb: Option<Arc<Image>>,
    pub meta: Meta,
    pub stage: Stage,
    /// The file on this Mac it came from, to offer again in "recent".
    pub origin: Option<PathBuf>,
    /// A recording being written down on this Mac; its words come as a text file.
    pub writing: bool,
    /// The picture whole on this Mac, for its big view (a paste kept in a
    /// scratch file; a file, where it is) — the thumbnail is chip-sized.
    pub whole: Option<PathBuf>,
}

impl Item {
    pub fn failed(&self) -> Option<&str> {
        match &self.stage {
            Stage::Failed(why) => Some(why),
            _ => None,
        }
    }
}

/// An attachment as a draft keeps it on disk (and the history, for ↑).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    pub kind: Kind,
    pub name: String,
    pub size: u64,
    #[serde(default)]
    pub meta: Meta,
    pub ready: Ready,
    /// The chip's PNG.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "base64_bytes")]
    pub thumb: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<PathBuf>,
}

mod base64_bytes {
    use base64::Engine as _;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        match bytes {
            Some(b) => s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(b)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
        let text: Option<String> = Option::deserialize(d)?;
        Ok(text.and_then(|t| base64::engine::general_purpose::STANDARD.decode(t).ok()))
    }
}

/// What is attached to the message being written.
#[derive(Debug, Default)]
pub struct Strip {
    items: Vec<Item>,
    next: u64,
}

impl Strip {
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// How many count toward the limit: all but the failed.
    fn counted(&self) -> usize {
        self.items.iter().filter(|i| i.failed().is_none()).count()
    }

    /// Room for one more?
    pub fn has_room(&self) -> bool {
        self.counted() < MAX_ATTACHMENTS
    }

    fn push(&mut self, item: Item) -> u64 {
        self.next += 1;
        let id = self.next;
        self.items.push(Item { id, ..item });
        id
    }

    /// One starts to be attached: its id, or `None` when the strip is full.
    pub fn begin(&mut self, source: &Source) -> Option<u64> {
        if !self.has_room() {
            return None;
        }
        Some(self.push(Item { id: 0, kind: source.kind(), name: source.name(), size: source.size(), thumb: None, meta: Meta::default(), stage: Stage::Preparing, origin: source.path().map(Path::to_owned), writing: false, whole: source.path().map(Path::to_owned) }))
    }

    /// One that can't be attached at all: shown as an error until dismissed.
    pub fn refuse(&mut self, name: &str, kind: Kind, why: String) -> u64 {
        self.push(Item { id: 0, kind, name: name.to_owned(), size: 0, thumb: None, meta: Meta::default(), stage: Stage::Failed(why), origin: None, writing: false, whole: None })
    }

    fn get(&mut self, id: u64) -> Option<&mut Item> {
        self.items.iter_mut().find(|i| i.id == id)
    }

    /// Read on this Mac: what its chip shows; now it is sent.
    pub fn prepared(&mut self, id: u64, prepared: &Prepared) {
        if let Some(item) = self.get(id) {
            item.kind = prepared.kind;
            item.name = prepared.name.clone();
            item.size = prepared.size;
            item.meta = prepared.meta.clone();
            item.thumb = prepared.thumb.clone().map(|png| Arc::new(Image::from_bytes(ImageFormat::Png, png)));
            item.stage = Stage::Sending(0.);
        }
    }

    pub fn progress(&mut self, id: u64, share: f32) -> bool {
        match self.get(id) {
            Some(item) if matches!(item.stage, Stage::Sending(_)) => {
                let before = item.stage.clone();
                item.stage = Stage::Sending((share * 20.).floor() / 20.);
                item.stage != before
            }
            _ => false,
        }
    }

    /// Kept on the daemon's machine — or failed, with why. One removed
    /// meanwhile is simply forgotten.
    pub fn finish(&mut self, id: u64, done: Result<Ready, String>) {
        if let Some(item) = self.get(id) {
            item.stage = match done {
                Ok(ready) => Stage::Ready(ready),
                Err(why) => Stage::Failed(why),
            };
        }
    }

    /// Its recording is being written down, or is done. False when it was taken off.
    pub fn writing(&mut self, id: u64, on: bool) -> bool {
        self.get(id).map(|item| item.writing = on).is_some()
    }

    /// Its picture whole, kept on this Mac.
    pub fn whole(&mut self, id: u64, path: PathBuf) {
        if let Some(item) = self.get(id) {
            item.whole = Some(path);
        }
    }

    pub fn remove(&mut self, id: u64) {
        self.items.retain(|i| i.id != id);
    }

    /// The last one goes (Backspace at the start of an empty box).
    pub fn pop(&mut self) -> Option<Item> {
        self.items.pop()
    }

    /// The failed ones are dismissed.
    pub fn dismiss_failed(&mut self) {
        self.items.retain(|i| i.failed().is_none());
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Something is still on its way: sending now would leave it behind.
    pub fn busy(&self) -> bool {
        self.items.iter().any(|i| i.writing || matches!(i.stage, Stage::Preparing | Stage::Sending(_)))
    }

    pub fn ready(&self) -> usize {
        self.items.iter().filter(|i| matches!(i.stage, Stage::Ready(_))).count()
    }

    pub fn failed(&self) -> usize {
        self.items.iter().filter(|i| i.failed().is_some()).count()
    }

    /// The tags of those ready, in order, each followed by a space.
    pub fn tags(&self) -> String {
        self.items
            .iter()
            .filter_map(|i| match &i.stage {
                Stage::Ready(ready) => Some(format!("{} ", ready.tag)),
                _ => None,
            })
            .collect()
    }

    /// The ones ready, as a draft keeps them.
    pub fn saved(&self) -> Vec<Saved> {
        self.items
            .iter()
            .filter_map(|i| match &i.stage {
                Stage::Ready(ready) => Some(Saved { kind: i.kind, name: i.name.clone(), size: i.size, meta: i.meta.clone(), ready: ready.clone(), thumb: i.thumb.as_ref().map(|t| t.bytes.clone()), origin: i.origin.clone() }),
                _ => None,
            })
            .collect()
    }

    /// Put back what a draft (or the history) kept, in place of what is here.
    pub fn restore(&mut self, saved: &[Saved]) {
        self.items.clear();
        for s in saved.iter().take(MAX_ATTACHMENTS) {
            let thumb = s.thumb.clone().map(|png| Arc::new(Image::from_bytes(ImageFormat::Png, png)));
            self.push(Item { id: 0, kind: s.kind, name: s.name.clone(), size: s.size, thumb, meta: s.meta.clone(), stage: Stage::Ready(s.ready.clone()), origin: s.origin.clone(), writing: false, whole: s.origin.clone() });
        }
    }

    /// Its number among the pictures (`[이미지 2]`), for a picture.
    pub fn picture_number(&self, id: u64) -> Option<usize> {
        self.items.iter().filter(|i| i.kind == Kind::Image).position(|i| i.id == id).map(|n| n + 1)
    }
}

/// The message to send: what was typed, then the attachments. A `/command`
/// carries none (they stay attached), and attachments alone are a message too.
pub fn message(typed: &str, tags: &str) -> String {
    let typed = typed.trim();
    if tags.is_empty() || typed.starts_with('/') {
        return typed.to_owned();
    }
    if typed.is_empty() { tags.trim_end().to_owned() } else { format!("{typed} {}", tags.trim_end()) }
}

/// What a sent message attached, as the strip keeps it, from its tags: for a
/// message taken back whose sending this app didn't see (another screen, a
/// restart). No thumbnail — the chip shows its kind.
pub fn saved_of(text: &str) -> Vec<Saved> {
    use equipe_core::protocol::tags::TagKind;
    equipe_core::protocol::tags::find(text)
        .into_iter()
        .map(|tag| {
            let name = tag.name().to_owned();
            let kind = match tag.kind {
                TagKind::Image => Kind::Image,
                TagKind::Video => Kind::Video,
                TagKind::Audio => Kind::Audio,
                TagKind::Pdf => Kind::Pdf,
                TagKind::File => match Kind::of(Path::new(&name)) {
                    k @ (Kind::Text | Kind::Doc) => k,
                    _ => Kind::File,
                },
            };
            let meta = Meta { pages: tag.pages, lines: tag.lines, dims: tag.dims, ..Meta::default() };
            Saved { kind, name, size: 0, meta, ready: Ready { path: tag.path.clone(), tag: text[tag.range.clone()].to_owned() }, thumb: None, origin: None }
        })
        .collect()
}

/// What was typed, without the characters that can't be seen or that turn
/// text around (zero-width spaces, bidi overrides, a stray BOM) — the joiners
/// emoji need stay.
pub fn clean(text: &str) -> String {
    text.chars().filter(|c| !matches!(c, '\u{200B}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}' | '\u{00AD}')).collect()
}

/// Whether a paste is long enough to travel as a file rather than in the box.
pub fn is_long_paste(text: &str) -> bool {
    text.chars().count() > PASTE_CHARS || text.lines().count() > PASTE_LINES
}

// ── what the clipboard and a drop hold ──

/// What a paste attaches: copied files first (any kind — like a drop), else a
/// picture (a screenshot). `None` when it is text — text wins over a picture.
pub fn from_clipboard(item: &gpui_kit::ClipboardItem) -> Option<Vec<Source>> {
    use gpui_kit::ClipboardEntry as E;
    let mut files = Vec::new();
    let mut pictures = Vec::new();
    let mut text = false;
    for entry in item.entries() {
        match entry {
            E::ExternalPaths(paths) => files.extend(paths.paths().iter().filter(|p| !p.is_dir()).cloned().map(Source::File)),
            E::Image(image) => pictures.push(Source::Bytes { name: "paste.png".into(), bytes: image.bytes.clone() }),
            E::String(s) => text |= !s.text.trim().is_empty(),
        }
    }
    if !files.is_empty() {
        return Some(files);
    }
    (!text && !pictures.is_empty()).then_some(pictures)
}

/// A dropped set of paths: the files to attach, and the folders (written into
/// the box as their paths).
pub fn from_drop(paths: &[PathBuf]) -> (Vec<Source>, Vec<PathBuf>) {
    let (folders, files): (Vec<_>, Vec<_>) = paths.iter().cloned().partition(|p| p.is_dir());
    (files.into_iter().map(Source::File).collect(), folders)
}

/// What a drag over the window would do, said before the drop: how many of
/// each kind are taken, and the first file refused and why.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Preview {
    pub taken: Vec<(Kind, usize)>,
    pub refused: Vec<(String, String)>,
    pub folders: usize,
    /// More than the strip has room for.
    pub over: usize,
}

pub fn preview(paths: &[PathBuf], room: usize) -> Preview {
    let mut out = Preview::default();
    let mut taken = 0;
    for path in paths {
        if path.is_dir() {
            out.folders += 1;
            continue;
        }
        match check(path) {
            Ok((kind, _)) if taken < room => {
                taken += 1;
                match out.taken.iter_mut().find(|(k, _)| *k == kind) {
                    Some((_, n)) => *n += 1,
                    None => out.taken.push((kind, 1)),
                }
            }
            Ok(_) => out.over += 1,
            Err(why) => out.refused.push((file_name(path), why)),
        }
    }
    out
}

// ── the inbox ──

/// Where `equipe send-image` on another machine drops images for this one.
pub fn inbox() -> PathBuf {
    equipe_providers::credentials::equipe_home().join("inbox")
}

/// Whether an image is waiting in the inbox.
pub fn inbox_waiting() -> bool {
    std::fs::read_dir(inbox()).is_ok_and(|entries| entries.flatten().any(|e| e.path().extension().is_some_and(|x| x == "png")))
}

/// What arrived in the inbox, taken out of it (a console on this machine may
/// take them too; whoever renames first has it).
pub fn take_inbox() -> Vec<Source> {
    let Ok(entries) = std::fs::read_dir(inbox()) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "png")).collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|path| {
            let bytes = std::fs::read(&path).ok()?;
            std::fs::remove_file(&path).ok()?;
            Some(Source::Bytes { name: path.file_name()?.to_string_lossy().into_owned(), bytes })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture_bytes(w: u32, h: u32, kind: Format) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(w, h).write_to(&mut out, kind).unwrap();
        out.into_inner()
    }

    fn dims(bytes: &[u8]) -> (u32, u32) {
        let image = image::load_from_memory(bytes).unwrap();
        (image.width(), image.height())
    }

    fn bytes_of(p: &Payload) -> &[u8] {
        match p {
            Payload::Bytes { bytes, .. } => bytes,
            Payload::File { .. } => panic!("a file"),
        }
    }

    fn dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-attach-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn ready(path: &str) -> Ready {
        Ready { path: path.into(), tag: format!("[image: {path}]") }
    }

    #[test]
    fn a_big_screenshot_is_shrunk_and_thumbnailed_small() {
        let prepared = prepare(Source::Bytes { name: "Screenshot.png".into(), bytes: picture_bytes(3200, 2000, Format::Png) }).unwrap();
        assert_eq!(prepared.name, "Screenshot.png");
        assert_eq!(dims(bytes_of(&prepared.main)), (1568, 980));
        let (w, h) = dims(prepared.thumb.as_ref().unwrap());
        assert!(w.max(h) <= THUMB, "the thumbnail is {w}x{h}");
        assert!(prepared.thumb.unwrap().len() < 64 << 10);
    }

    #[test]
    fn a_small_picture_is_kept_as_it_came_and_tiff_becomes_png() {
        let bytes = picture_bytes(300, 200, Format::Jpeg);
        let prepared = prepare(Source::Bytes { name: "a.jpeg".into(), bytes: bytes.clone() }).unwrap();
        assert_eq!(prepared.name, "a.jpg");
        assert_eq!(bytes_of(&prepared.main), bytes.as_slice());
        let tiff = prepare(Source::Bytes { name: "paste.png".into(), bytes: picture_bytes(40, 40, Format::Tiff) }).unwrap();
        assert_eq!(image::guess_format(bytes_of(&tiff.main)).unwrap(), Format::Png);
        assert!(prepare(Source::Bytes { name: "x.png".into(), bytes: b"not a picture".to_vec() }).is_err());
    }

    #[test]
    fn kinds_come_from_the_name() {
        let k = |n: &str| Kind::of(Path::new(n));
        assert_eq!([k("a.PNG"), k("clip.mov"), k("memo.m4a"), k("spec.pdf"), k("main.rs"), k("Makefile"), k("plan.docx"), k("x.bin")], [
            Kind::Image,
            Kind::Video,
            Kind::Audio,
            Kind::Pdf,
            Kind::Text,
            Kind::Text,
            Kind::Doc,
            Kind::File
        ]);
        assert_eq!(badge("plan.docx", Kind::Doc), "DOCX");
        assert_eq!(badge("archive.tar.gzipped", Kind::File), "FILE");
        assert_eq!(badge("README", Kind::Text), "TXT");
        assert_eq!(Kind::Image.upload(), None, "a picture uploads as any daemon expects");
        assert_eq!(Kind::Text.upload(), Some(UploadKind::File));
    }

    #[test]
    fn a_file_is_checked_before_it_is_taken() {
        let dir = dir("check");
        std::fs::write(dir.join("notes.md"), "# a\nb\n").unwrap();
        std::fs::write(dir.join("empty.txt"), "").unwrap();
        assert_eq!(check(&dir.join("notes.md")), Ok((Kind::Text, 6)));
        assert!(check(&dir).unwrap_err().contains("폴더"));
        assert!(check(&dir.join("missing.pdf")).is_err());
        assert!(check(&dir.join("empty.txt")).is_err());
        let p = preview(&[dir.join("notes.md"), dir.join("missing.pdf"), dir.clone()], 20);
        assert_eq!(p.taken, vec![(Kind::Text, 1)]);
        assert_eq!((p.refused.len(), p.folders), (1, 1));
        assert_eq!(preview(&[dir.join("notes.md")], 0).over, 1, "no room left");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn text_counts_its_lines_and_a_long_paste_is_a_file() {
        let dir = dir("text");
        std::fs::write(dir.join("a.rs"), "fn main() {\n}\nlast").unwrap();
        let prepared = prepare(Source::File(dir.join("a.rs"))).unwrap();
        assert_eq!((prepared.kind, prepared.meta.lines), (Kind::Text, Some(3)));
        let long = "줄\n".repeat(41);
        assert!(is_long_paste(&long) && !is_long_paste("짧은 글"));
        let pasted = prepare(Source::Text { name: "pasted.txt".into(), text: long }).unwrap();
        assert_eq!(pasted.meta.lines, Some(41));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn on_this_mac_a_file_is_cloned_into_the_daemons_uploads_and_tagged() {
        let dir = dir("local");
        let home = dir.join("home");
        std::fs::write(dir.join("기획서.md"), "# 기획\n").unwrap();
        let prepared = prepare(Source::File(dir.join("기획서.md"))).unwrap();
        let store = Store::Local { home: home.clone() };
        let seen = std::cell::Cell::new(0.);
        let ready = deliver(&prepared, &store, &|f| seen.set(f)).unwrap();
        assert!(ready.path.starts_with(&home.join("uploads").to_string_lossy().into_owned()), "{ready:?}");
        assert!(ready.path.ends_with("-기획서.md"));
        assert_eq!(std::fs::read_to_string(&ready.path).unwrap(), "# 기획\n");
        assert_eq!(ready.tag, format!("[file: {} · 1 line]", ready.path));
        assert_eq!(seen.get(), 1.);
        // The original stays where it was.
        assert!(dir.join("기획서.md").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn tags_say_what_a_member_needs() {
        let meta = Meta { pages: Some(12), ..Meta::default() };
        assert_eq!(tag(Kind::Pdf, "/u/spec.pdf", &meta, &[], None), "[pdf: /u/spec.pdf · 12 pages]");
        let video = Meta { duration: Some(102.), dims: Some((1920, 1080)), frames: 2, ..Meta::default() };
        assert_eq!(
            tag(Kind::Video, "/u/a.mov", &video, &[(3., "/u/f1.jpg".into()), (15., "/u/f2.jpg".into())], Some("/u/s.jpg")),
            "[video: /u/a.mov · 1:42 · 1920×1080 · sheet: /u/s.jpg · frames: /u/f1.jpg@0:03, /u/f2.jpg@0:15]"
        );
        assert_eq!(tag(Kind::Audio, "/u/m.m4a", &Meta { duration: Some(31.), ..Meta::default() }, &[], None), "[audio: /u/m.m4a · 0:31]");
        assert_eq!(tag(Kind::Doc, "/u/x.docx", &Meta::default(), &[], None), "[file: /u/x.docx]");
        assert_eq!(tag(Kind::Image, "/u/a.png", &Meta { dims: Some((3, 3)), ..Meta::default() }, &[], None), "[image: /u/a.png]");
    }

    #[test]
    fn the_strip_holds_up_to_its_limit_and_names_them_for_the_message() {
        let mut strip = Strip::default();
        let source = |n: usize| Source::Bytes { name: format!("{n}.png"), bytes: vec![] };
        let ids: Vec<u64> = (0..MAX_ATTACHMENTS).map(|n| strip.begin(&source(n)).unwrap()).collect();
        assert!(strip.begin(&source(99)).is_none());
        assert!(strip.busy() && strip.tags().is_empty());
        for (n, id) in ids.iter().enumerate() {
            strip.finish(*id, Ok(ready(&format!("/up/{n}.png"))));
        }
        assert!(!strip.busy());
        assert_eq!(strip.ready(), MAX_ATTACHMENTS);
        assert!(strip.tags().starts_with("[image: /up/0.png] [image: /up/1.png] "));
        strip.remove(ids[0]);
        assert!(strip.has_room() && strip.begin(&source(1)).is_some());
    }

    #[test]
    fn one_that_fails_stays_with_its_reason_until_dismissed_and_does_not_count() {
        let mut strip = Strip::default();
        let (a, b) = (strip.begin(&Source::Text { name: "a".into(), text: "x".into() }).unwrap(), strip.begin(&Source::Text { name: "b".into(), text: "y".into() }).unwrap());
        strip.finish(a, Err("too big".into()));
        assert_eq!(strip.items()[0].failed(), Some("too big"));
        assert_eq!((strip.failed(), strip.len()), (1, 2));
        strip.remove(b);
        // `b` finishing later changes nothing.
        strip.finish(b, Ok(ready("/up/b.png")));
        assert_eq!(strip.len(), 1);
        strip.refuse("big.mov", Kind::Video, "2 GB까지".into());
        for n in 0..MAX_ATTACHMENTS {
            assert!(strip.begin(&Source::Text { name: format!("{n}"), text: "x".into() }).is_some(), "failed ones leave room");
        }
        strip.dismiss_failed();
        assert_eq!(strip.failed(), 0);
    }

    #[test]
    fn a_recording_being_written_down_holds_the_send() {
        let mut strip = Strip::default();
        let memo = strip.begin(&Source::File("/u/memo.m4a".into())).unwrap();
        assert!(strip.writing(memo, true));
        strip.finish(memo, Ok(ready("/up/memo.m4a")));
        assert!(strip.busy(), "sent, but its words are still coming");
        strip.writing(memo, false);
        assert!(!strip.busy());
        strip.remove(memo);
        assert!(!strip.writing(memo, false), "taken off: its words aren't wanted");
    }

    #[test]
    fn a_draft_keeps_the_ready_ones_with_their_thumbnails() {
        let mut strip = Strip::default();
        let id = strip.begin(&Source::Bytes { name: "a.png".into(), bytes: picture_bytes(10, 10, Format::Png) }).unwrap();
        let prepared = prepare(Source::Bytes { name: "a.png".into(), bytes: picture_bytes(10, 10, Format::Png) }).unwrap();
        strip.prepared(id, &prepared);
        assert!(strip.progress(id, 0.5));
        assert!(!strip.progress(id, 0.51), "small steps don't redraw");
        strip.finish(id, Ok(ready("/up/a.png")));
        let saved = strip.saved();
        let json = serde_json::to_string(&saved).unwrap();
        let back: Vec<Saved> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, saved);
        let mut other = Strip::default();
        other.restore(&back);
        assert_eq!(other.tags(), "[image: /up/a.png] ");
        assert!(other.items()[0].thumb.is_some());
        assert_eq!(other.picture_number(other.items()[0].id), Some(1));
        assert!(other.pop().is_some() && other.is_empty());
    }

    #[test]
    fn the_message_is_the_words_then_the_attachments() {
        assert_eq!(message(" look at this ", "[image: /a.png] "), "look at this [image: /a.png]");
        assert_eq!(message("", "[image: /a.png] "), "[image: /a.png]");
        assert_eq!(message("/board", "[image: /a.png] "), "/board");
        assert_eq!(message("hi", ""), "hi");
    }

    #[test]
    fn invisible_and_bidi_characters_are_dropped_but_emoji_joiners_stay() {
        assert_eq!(clean("a\u{200B}b\u{202E}c\u{FEFF}"), "abc");
        assert_eq!(clean("👨\u{200D}💻"), "👨\u{200D}💻");
    }

    #[test]
    fn the_clipboard_gives_files_then_pictures_unless_text_is_there() {
        use gpui_kit::ClipboardItem;
        let shot = Image::from_bytes(ImageFormat::Png, picture_bytes(8, 8, Format::Png));
        assert_eq!(from_clipboard(&ClipboardItem::new_image(&shot)).map(|s| s.len()), Some(1));
        assert!(from_clipboard(&ClipboardItem::new_string("hello".into())).is_none());
        let dir = dir("drop");
        std::fs::write(dir.join("a.pdf"), "x").unwrap();
        let (files, folders) = from_drop(&[dir.join("a.pdf"), dir.clone()]);
        assert_eq!((files.len(), folders), (1, vec![dir.clone()]));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
