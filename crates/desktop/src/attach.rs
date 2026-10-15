//! Images attached to a message: what is allowed, how a picture is made ready
//! (decoded, shrunk, thumbnailed — all off the UI thread), the strip of what is
//! attached so far, and how the message that carries them reads. Like the
//! terminal console, an image reaches a member as a file on the daemon's
//! machine plus `[image: <path>] ` in the message; the protocol has no image field.

use equipe_core::t;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::{Image, ImageFormat};
use image::ImageFormat as Kind;

/// How many images one message may carry.
pub const MAX_IMAGES: usize = 20;
/// The largest one image may be once ready, in bytes.
pub const MAX_BYTES: usize = 8 << 20;
/// A longer side than this is shrunk before it is kept (what a member can use).
pub const LONG_SIDE: u32 = 1568;
/// The thumbnail's side in pixels: 72 points at 2x.
pub const THUMB: u32 = 144;
/// A file bigger than this isn't even read.
const READ_MAX: u64 = 64 << 20;

/// Where an image comes from.
#[derive(Debug, Clone)]
pub enum Source {
    /// The clipboard, a drop from another app, the inbox: bytes already in hand.
    Bytes { name: String, bytes: Vec<u8> },
    /// A file dragged in or copied in Finder.
    File(PathBuf),
}

impl Source {
    pub fn name(&self) -> String {
        match self {
            Self::Bytes { name, .. } => name.clone(),
            Self::File(path) => path.file_name().map_or_else(|| "image".into(), |n| n.to_string_lossy().into_owned()),
        }
    }
}

/// An image made ready to keep and show.
#[derive(Debug)]
pub struct Prepared {
    /// What the daemon is to keep (its extension is what counts).
    pub name: String,
    pub bytes: Vec<u8>,
    /// A small PNG to show in the strip.
    pub thumb: Vec<u8>,
}

/// Whether a path names an image we take, by its extension.
pub fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "tiff" | "tif"))
}

fn extension(kind: Kind) -> Option<&'static str> {
    match kind {
        Kind::Png => Some("png"),
        Kind::Jpeg => Some("jpg"),
        Kind::Gif => Some("gif"),
        Kind::WebP => Some("webp"),
        _ => None,
    }
}

fn png(image: &image::DynamicImage) -> Result<Vec<u8>, String> {
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, Kind::Png).map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

/// Decode, shrink what is too long, thumbnail. Blocking, and the decoded
/// picture is gone when it returns: only the bytes to keep and a small PNG stay.
pub fn prepare(source: Source) -> Result<Prepared, String> {
    let (name, bytes) = match source {
        Source::Bytes { name, bytes } => (name, bytes),
        Source::File(path) => {
            let name = Source::File(path.clone()).name();
            let big = std::fs::metadata(&path).map_err(|_| t!("attach.unreadable"))?.len() > READ_MAX;
            if big {
                return Err(t!("attach.too_big_file", name = name));
            }
            (name, std::fs::read(&path).map_err(|_| t!("attach.unreadable"))?)
        }
    };
    let kind = image::guess_format(&bytes).map_err(|_| t!("attach.unreadable").to_owned())?;
    let decoded = image::load_from_memory_with_format(&bytes, kind).map_err(|_| t!("attach.unreadable").to_owned())?;
    let thumb = png(&decoded.thumbnail(THUMB, THUMB))?;
    let shrink = decoded.width().max(decoded.height()) > LONG_SIDE;
    let (name, bytes) = match (shrink, extension(kind)) {
        // Too long, or a format the daemon doesn't take (TIFF from Preview): as PNG.
        (true, _) | (false, None) => {
            let kept = if shrink { decoded.resize(LONG_SIDE, LONG_SIDE, image::imageops::FilterType::Triangle) } else { decoded };
            let stem = Path::new(&name).file_stem().map_or_else(|| "image".into(), |s| s.to_string_lossy().into_owned());
            (format!("{stem}.png"), png(&kept)?)
        }
        (false, Some(ext)) => {
            let stem = Path::new(&name).file_stem().map_or_else(|| "image".into(), |s| s.to_string_lossy().into_owned());
            (format!("{stem}.{ext}"), bytes)
        }
    };
    if bytes.len() > MAX_BYTES {
        return Err(t!("attach.too_big", mb = MAX_BYTES >> 20));
    }
    Ok(Prepared { name, bytes, thumb })
}

// ── the strip ──

/// Where one attached image stands.
#[derive(Debug)]
pub enum Stage {
    /// Being made ready and sent to the daemon's machine.
    Working,
    /// Kept there, at this path.
    Ready(String),
}

#[derive(Debug)]
pub struct Item {
    pub id: u64,
    pub name: String,
    pub size: usize,
    pub thumb: Option<Arc<Image>>,
    pub stage: Stage,
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

    /// Room for one more?
    pub fn has_room(&self) -> bool {
        self.items.len() < MAX_IMAGES
    }

    /// An image starts to be attached: its id, or `None` when the strip is full.
    pub fn begin(&mut self, name: &str) -> Option<u64> {
        if !self.has_room() {
            return None;
        }
        self.next += 1;
        self.items.push(Item { id: self.next, name: name.to_owned(), size: 0, thumb: None, stage: Stage::Working });
        Some(self.next)
    }

    /// An image is ready (or failed: it leaves, and the reason comes back).
    /// An image removed meanwhile is simply forgotten.
    pub fn finish(&mut self, id: u64, done: Result<(Prepared, String), String>) -> Option<String> {
        let at = self.items.iter().position(|i| i.id == id)?;
        match done {
            Ok((prepared, path)) => {
                let item = &mut self.items[at];
                item.name = prepared.name;
                item.size = prepared.bytes.len();
                item.thumb = Some(Arc::new(Image::from_bytes(ImageFormat::Png, prepared.thumb)));
                item.stage = Stage::Ready(path);
                None
            }
            Err(why) => {
                self.items.remove(at);
                Some(why)
            }
        }
    }

    pub fn remove(&mut self, id: u64) {
        self.items.retain(|i| i.id != id);
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Something is still on its way: sending now would leave it behind.
    pub fn busy(&self) -> bool {
        self.items.iter().any(|i| matches!(i.stage, Stage::Working))
    }

    pub fn ready(&self) -> usize {
        self.items.iter().filter(|i| matches!(i.stage, Stage::Ready(_))).count()
    }

    /// `[image: <path>] ` for each, as the terminal console writes them.
    pub fn tags(&self) -> String {
        self.items
            .iter()
            .filter_map(|i| match &i.stage {
                Stage::Ready(path) => Some(format!("[image: {path}] ")),
                Stage::Working => None,
            })
            .collect()
    }
}

/// The message to send: what was typed, then the images. A `/command` carries
/// no images (they stay attached), and images alone are a message too.
pub fn message(typed: &str, tags: &str) -> String {
    let typed = typed.trim();
    if tags.is_empty() || typed.starts_with('/') {
        return typed.to_owned();
    }
    if typed.is_empty() { tags.to_owned() } else { format!("{typed} {tags}") }
}

// ── what the clipboard and a drop hold ──

/// The images in a pasted clipboard: copied image files first, else a picture
/// (a screenshot). `None` when it is text — text wins when both are there,
/// and a copied non-image file is text too.
pub fn from_clipboard(item: &gpui_kit::ClipboardItem) -> Option<Vec<Source>> {
    use gpui_kit::ClipboardEntry as E;
    let mut files = Vec::new();
    let mut pictures = Vec::new();
    let mut text = false;
    for entry in item.entries() {
        match entry {
            E::ExternalPaths(paths) => files.extend(paths.paths().iter().filter(|p| is_image_path(p)).cloned().map(Source::File)),
            E::Image(image) => pictures.push(Source::Bytes { name: "paste.png".into(), bytes: image.bytes.clone() }),
            E::String(s) => text |= !s.text.trim().is_empty(),
        }
    }
    if !files.is_empty() {
        return Some(files);
    }
    (!text && !pictures.is_empty()).then_some(pictures)
}

/// A dropped set of paths: the images, and the rest to be written as text.
pub fn from_drop(paths: &[PathBuf]) -> (Vec<Source>, Vec<PathBuf>) {
    let (images, rest): (Vec<_>, Vec<_>) = paths.iter().cloned().partition(|p| is_image_path(p) && p.is_file());
    (images.into_iter().map(Source::File).collect(), rest)
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

    fn picture(w: u32, h: u32, kind: Kind) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(w, h).write_to(&mut out, kind).unwrap();
        out.into_inner()
    }

    fn dims(bytes: &[u8]) -> (u32, u32) {
        let image = image::load_from_memory(bytes).unwrap();
        (image.width(), image.height())
    }

    fn ready(name: &str) -> Prepared {
        Prepared { name: name.into(), bytes: vec![0; 10], thumb: picture(4, 4, Kind::Png) }
    }

    #[test]
    fn a_big_screenshot_is_shrunk_and_thumbnailed_small() {
        let prepared = prepare(Source::Bytes { name: "Screenshot.png".into(), bytes: picture(3200, 2000, Kind::Png) }).unwrap();
        assert_eq!(prepared.name, "Screenshot.png");
        assert_eq!(dims(&prepared.bytes), (1568, 980));
        let (w, h) = dims(&prepared.thumb);
        assert!(w.max(h) <= THUMB, "the thumbnail is {w}x{h}");
        // What stays in memory is the small PNG, not a decoded picture.
        assert!(prepared.thumb.len() < 64 << 10);
    }

    #[test]
    fn a_small_picture_is_kept_as_it_came() {
        let bytes = picture(300, 200, Kind::Jpeg);
        let prepared = prepare(Source::Bytes { name: "a.jpeg".into(), bytes: bytes.clone() }).unwrap();
        assert_eq!(prepared.name, "a.jpg");
        assert_eq!(prepared.bytes, bytes);
    }

    #[test]
    fn a_format_the_daemon_does_not_take_becomes_a_png() {
        let prepared = prepare(Source::Bytes { name: "paste.png".into(), bytes: picture(40, 40, Kind::Tiff) }).unwrap();
        assert_eq!(image::guess_format(&prepared.bytes).unwrap(), Kind::Png);
    }

    #[test]
    fn what_is_not_a_picture_is_refused() {
        assert!(prepare(Source::Bytes { name: "x.png".into(), bytes: b"not a picture".to_vec() }).is_err());
        assert!(prepare(Source::File("/nonexistent/x.png".into())).is_err());
    }

    #[test]
    fn a_file_is_read_by_its_path() {
        let dir = std::env::temp_dir().join(format!("equipe-attach-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shot.png");
        std::fs::write(&path, picture(20, 20, Kind::Png)).unwrap();
        let (images, rest) = from_drop(&[path.clone(), dir.join("notes.txt")]);
        assert_eq!((images.len(), rest), (1, vec![dir.join("notes.txt")]));
        assert_eq!(prepare(images.into_iter().next().unwrap()).unwrap().name, "shot.png");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_strip_holds_up_to_its_limit_and_names_them_for_the_message() {
        let mut strip = Strip::default();
        let ids: Vec<u64> = (0..MAX_IMAGES).map(|n| strip.begin(&format!("{n}.png")).unwrap()).collect();
        assert!(strip.begin("6.png").is_none());
        assert!(strip.busy() && strip.tags().is_empty());
        for (n, id) in ids.iter().enumerate() {
            assert_eq!(strip.finish(*id, Ok((ready("a.png"), format!("/up/{n}.png")))), None);
        }
        assert!(!strip.busy());
        assert_eq!(strip.ready(), MAX_IMAGES);
        assert!(strip.tags().starts_with("[image: /up/0.png] [image: /up/1.png] "));
        strip.remove(ids[0]);
        assert!(strip.has_room() && strip.begin("again.png").is_some());
    }

    #[test]
    fn one_that_fails_leaves_with_its_reason_and_a_removed_one_is_forgotten() {
        let mut strip = Strip::default();
        let (a, b) = (strip.begin("a.png").unwrap(), strip.begin("b.png").unwrap());
        assert_eq!(strip.finish(a, Err("too big".into())).as_deref(), Some("too big"));
        strip.remove(b);
        // The upload of `b` finishing later changes nothing.
        assert_eq!(strip.finish(b, Ok((ready("b.png"), "/up/b.png".into()))), None);
        assert!(strip.is_empty());
    }

    #[test]
    fn the_message_is_the_words_then_the_images() {
        assert_eq!(message(" look at this ", "[image: /a.png] "), "look at this [image: /a.png] ");
        assert_eq!(message("", "[image: /a.png] "), "[image: /a.png] ");
        assert_eq!(message("@frontend fix", "[image: /a.png] "), "@frontend fix [image: /a.png] ");
        assert_eq!(message("/board", "[image: /a.png] "), "/board");
        assert_eq!(message("hi", ""), "hi");
    }

    #[test]
    fn the_clipboard_gives_pictures_unless_text_is_there() {
        use gpui_kit::ClipboardItem;
        let shot = Image::from_bytes(ImageFormat::Png, picture(8, 8, Kind::Png));
        assert_eq!(from_clipboard(&ClipboardItem::new_image(&shot)).map(|s| s.len()), Some(1));
        assert!(from_clipboard(&ClipboardItem::new_string("hello".into())).is_none());
    }
}
