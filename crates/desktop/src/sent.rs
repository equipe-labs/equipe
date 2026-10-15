//! What a sent message attached, shown as itself — pictures as pictures, a
//! video as its first frame, a PDF or a file as a chip — the way the tags in
//! its text (`equipe_core::protocol::tags`) point at them.
//!
//! The files live on the daemon's machine. Right after sending, the app still
//! has what it sent and keeps it here at once, by the path it was kept under;
//! later (a restart, another Mac) a daemon on this Mac's file is read as it
//! is, one on another machine's is asked for (`read_upload`, which gives
//! nothing outside its uploads). Each picture is kept as a small thumbnail —
//! and, from another machine, whole for the lightbox, within a budget.
//! No UI types beyond the image here, so the rules can be tested.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use equipe_core::protocol::tags::{Tag, TagKind};
use gpui_kit::{Image, ImageFormat};

use crate::attach::{Payload, Prepared, Ready};

/// How many attachments of one message are shown.
pub const MAX_SHOWN: usize = 20;
/// The tallest one picture alone is shown, in points.
pub const TALL: f32 = 240.;
/// The widest one picture alone is shown.
pub const WIDE: f32 = 420.;
/// The side of each picture when there are several.
pub const TILE: f32 = 112.;
/// The size a picture not yet known holds.
pub const UNKNOWN: (f32, f32) = (220., 150.);
/// A thumbnail is made for 2x: within this box, in pixels.
const THUMB_W: u32 = 960;
const THUMB_H: u32 = 480;
/// Pictures kept whole, for the lightbox, at most this many bytes all told.
const FULL_BUDGET: usize = 64 << 20;
/// The most read from a file on this Mac (as the daemon gives at most).
const READ_MAX: u64 = equipe_core::gateway::READ_UPLOAD_MAX;

/// What is known of one file a message attached.
#[derive(Debug, Clone)]
pub enum Shown {
    /// Asked for, not here yet.
    Loading,
    Picture(Picture),
    /// A file that isn't a picture: how big it is.
    Sized(u64),
    /// Not there any more (uploads are swept after a week), or not readable.
    Gone,
}

#[derive(Debug, Clone)]
pub struct Picture {
    pub thumb: Arc<Image>,
    /// Its size in pixels.
    pub dims: (u32, u32),
    /// Whole, when it came from another machine (on this Mac the file is read).
    pub full: Option<Arc<Image>>,
    pub size: u64,
    /// When it arrived: it fades in for a moment after.
    pub at: Instant,
}

/// What a row needs of a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Want {
    Picture,
    Size,
}

/// Every attachment file the conversation has asked about, by its path on
/// the daemon's machine.
#[derive(Debug, Default)]
pub struct Sent {
    shown: HashMap<String, Shown>,
    wanted: Vec<(String, Want)>,
    /// A load of what is wanted is on its way to being started.
    pub scheduled: bool,
    /// Pictures kept whole, oldest first, and their bytes all told.
    whole: VecDeque<String>,
    whole_bytes: usize,
    /// Counts up when the machine changes: loads started before don't land.
    pub generation: u64,
}

impl Sent {
    pub fn get(&self, path: &str) -> Option<&Shown> {
        self.shown.get(path)
    }

    /// What `path` shows now; the first time, it is asked for.
    pub fn look(&mut self, path: &str, want: Want) -> Shown {
        if let Some(shown) = self.shown.get(path) {
            return shown.clone();
        }
        self.shown.insert(path.to_owned(), Shown::Loading);
        self.wanted.push((path.to_owned(), want));
        Shown::Loading
    }

    /// What is to be loaded, taken.
    pub fn take_wanted(&mut self) -> Vec<(String, Want)> {
        self.scheduled = false;
        std::mem::take(&mut self.wanted)
    }

    /// What `path` shows from now on. Whole pictures past the budget are let
    /// go, oldest first (their thumbnails stay).
    pub fn put(&mut self, path: &str, shown: Shown) {
        if let Some(Shown::Picture(Picture { full: Some(full), .. })) = self.shown.get(path) {
            self.whole_bytes = self.whole_bytes.saturating_sub(full.bytes.len());
            self.whole.retain(|p| p != path);
        }
        if let Shown::Picture(Picture { full: Some(full), .. }) = &shown {
            self.whole_bytes += full.bytes.len();
            self.whole.push_back(path.to_owned());
        }
        self.shown.insert(path.to_owned(), shown);
        while self.whole_bytes > FULL_BUDGET && self.whole.len() > 1 {
            let Some(oldest) = self.whole.pop_front() else { break };
            if let Some(Shown::Picture(picture)) = self.shown.get_mut(&oldest)
                && let Some(full) = picture.full.take()
            {
                self.whole_bytes = self.whole_bytes.saturating_sub(full.bytes.len());
            }
        }
    }

    /// Kept right after sending: shown at once, never asked for.
    pub fn seed(&mut self, path: &str, shown: Shown) {
        let better = !matches!(self.shown.get(path), Some(Shown::Picture(_)));
        if better {
            self.put(path, shown);
        }
    }

    /// Back on the line: what couldn't be had is asked for again.
    pub fn retry_gone(&mut self) {
        self.shown.retain(|_, s| !matches!(s, Shown::Gone));
    }

    /// Another machine: nothing known holds.
    pub fn clear(&mut self) {
        *self = Self { generation: self.generation + 1, ..Self::default() };
    }

    #[cfg(test)]
    fn whole_bytes(&self) -> usize {
        self.whole_bytes
    }
}

/// What a message shows of its attachments: the words without the tags, and
/// the tags (at most [`MAX_SHOWN`]), with how many more there were.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Parts {
    pub text: String,
    pub tags: Vec<Tag>,
    pub more: usize,
}

pub fn parts(text: &str) -> Parts {
    let mut split = equipe_core::protocol::tags::split(text);
    let more = split.tags.len().saturating_sub(MAX_SHOWN);
    split.tags.truncate(MAX_SHOWN);
    Parts { text: split.text, tags: split.tags, more }
}

/// A message in one line's words: each attachment as its kind and name
/// (`[이미지] paste.png`), for where it is named rather than shown.
pub fn summary(text: &str) -> String {
    equipe_core::protocol::tags::compact(text, |tag| {
        let name = tag.name();
        match tag.kind {
            TagKind::Image => equipe_core::t!("sent.label_image", name = name),
            TagKind::Video => equipe_core::t!("sent.label_video", name = name),
            TagKind::Audio => equipe_core::t!("sent.label_audio", name = name),
            TagKind::Pdf => equipe_core::t!("sent.label_pdf", name = name),
            TagKind::File => equipe_core::t!("sent.label_file", name = name),
        }
    })
}

/// What a tag needs loaded, and from where.
pub fn want_of(tag: &Tag) -> Option<(&str, Want)> {
    match tag.kind {
        TagKind::Image | TagKind::Video => tag.picture().map(|p| (p, Want::Picture)),
        TagKind::Pdf | TagKind::File | TagKind::Audio => Some((tag.path.as_str(), Want::Size)),
    }
}

/// The pictures of a message, in order, that the lightbox goes through: an
/// image, and a video's sheet (else its first frame).
pub fn gallery_of(tags: &[Tag]) -> Vec<String> {
    tags.iter()
        .filter_map(|t| match t.kind {
            TagKind::Image => Some(t.path.clone()),
            TagKind::Video => t.sheet.clone().or_else(|| t.picture().map(str::to_owned)),
            _ => None,
        })
        .collect()
}

/// Pictures shown big, one at a time, ←/→ going through them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gallery {
    pub paths: Vec<String>,
    pub at: usize,
}

impl Gallery {
    /// Open at `path` among `paths`.
    pub fn open(paths: Vec<String>, path: &str) -> Option<Self> {
        let at = paths.iter().position(|p| p == path)?;
        Some(Self { paths, at })
    }

    pub fn step(&mut self, forward: bool) {
        let n = self.paths.len().max(1);
        self.at = if forward { (self.at + 1) % n } else { (self.at + n - 1) % n };
    }

    pub fn path(&self) -> &str {
        &self.paths[self.at]
    }
}

/// How big one picture alone is drawn, in points: its pixels as points (a
/// 2x screenshot at its size), no taller than [`TALL`] nor wider than
/// [`WIDE`] or `room`, its shape kept; a picture not yet known holds [`UNKNOWN`].
pub fn single_size(dims: Option<(u32, u32)>, room: f32) -> (f32, f32) {
    let Some((w, h)) = dims.filter(|(w, h)| *w > 0 && *h > 0) else {
        return (UNKNOWN.0.min(room), UNKNOWN.1);
    };
    let (w, h) = (w as f32 / 2., h as f32 / 2.);
    let scale = (TALL / h).min(WIDE.min(room) / w).min(1.);
    ((w * scale).max(24.).round(), (h * scale).max(24.).round())
}

// ── loading ──

/// Where the files are: on this Mac, or asked for over the line.
#[derive(Clone)]
pub enum Where {
    Local,
    Remote(crate::daemon::Uploader),
}

/// Load what a row wants of `path`. Blocking: off the UI thread.
pub fn load(from: &Where, path: &str, want: Want) -> Shown {
    match from {
        Where::Local => match want {
            Want::Size => std::fs::metadata(path).ok().filter(|m| m.is_file()).map_or(Shown::Gone, |m| Shown::Sized(m.len())),
            Want::Picture => {
                use std::io::Read as _;
                let mut bytes = Vec::new();
                let read = std::fs::File::open(path).and_then(|f| f.take(READ_MAX).read_to_end(&mut bytes));
                match read {
                    Ok(_) => picture(&bytes, false).map_or(Shown::Gone, Shown::Picture),
                    Err(_) => Shown::Gone,
                }
            }
        },
        Where::Remote(uploader) => match uploader.read(path, want == Want::Size) {
            Ok(read) => match (want, read.data) {
                (Want::Size, _) => Shown::Sized(read.size),
                (Want::Picture, Some(data)) => {
                    use base64::Engine as _;
                    base64::engine::general_purpose::STANDARD.decode(data).ok().and_then(|bytes| picture(&bytes, true)).map_or(Shown::Gone, Shown::Picture)
                }
                (Want::Picture, None) => Shown::Gone,
            },
            Err(_) => Shown::Gone,
        },
    }
}

/// A picture's thumbnail (and itself whole, when `keep_whole`), or `None`
/// for bytes that aren't one.
pub fn picture(bytes: &[u8], keep_whole: bool) -> Option<Picture> {
    let format = image::guess_format(bytes).ok()?;
    let decoded = image::load_from_memory_with_format(bytes, format).ok()?;
    let dims = (decoded.width(), decoded.height());
    let small = if dims.0 > THUMB_W || dims.1 > THUMB_H { decoded.thumbnail(THUMB_W, THUMB_H) } else { decoded };
    let thumb = if small.color().has_alpha() {
        let mut out = std::io::Cursor::new(Vec::new());
        small.to_rgba8().write_to(&mut out, image::ImageFormat::Png).ok()?;
        Image::from_bytes(ImageFormat::Png, out.into_inner())
    } else {
        Image::from_bytes(ImageFormat::Jpeg, crate::media::jpeg(&small.to_rgb8()).ok()?)
    };
    let whole = match format {
        _ if !keep_whole => None,
        image::ImageFormat::Png => Some(ImageFormat::Png),
        image::ImageFormat::Jpeg => Some(ImageFormat::Jpeg),
        image::ImageFormat::Gif => Some(ImageFormat::Gif),
        image::ImageFormat::WebP => Some(ImageFormat::Webp),
        image::ImageFormat::Bmp => Some(ImageFormat::Bmp),
        image::ImageFormat::Tiff => Some(ImageFormat::Tiff),
        _ => None,
    };
    let full = whole.map(|f| Arc::new(Image::from_bytes(f, bytes.to_vec())));
    Some(Picture { thumb: Arc::new(thumb), dims, full, size: bytes.len() as u64, at: Instant::now() })
}

/// What was just sent, shown at once from what the app still has: a
/// picture's bytes, a video's first frame, any file's size — by the paths
/// its tag names.
pub fn just_sent(prepared: &Prepared, ready: &Ready, keep_whole: bool) -> Vec<(String, Shown)> {
    let mut out = vec![];
    let Some(tag) = equipe_core::protocol::tags::find(&ready.tag).into_iter().next() else {
        return out;
    };
    let bytes = |p: &Payload| match p {
        Payload::Bytes { bytes, .. } => Some(bytes.clone()),
        Payload::File { path, .. } => std::fs::metadata(path).ok().filter(|m| m.len() <= READ_MAX).and_then(|_| std::fs::read(path).ok()),
    };
    match tag.kind {
        TagKind::Image => {
            if let Some(picture) = bytes(&prepared.main).and_then(|b| picture(&b, keep_whole)) {
                out.push((tag.path.clone(), Shown::Picture(picture)));
            }
        }
        TagKind::Video => {
            if let (Some((first, _)), Some((_, frame))) = (tag.frames.first(), prepared.frames.first())
                && let Some(picture) = bytes(frame).and_then(|b| picture(&b, keep_whole))
            {
                out.push((first.clone(), Shown::Picture(picture)));
            }
            out.push((tag.path.clone(), Shown::Sized(prepared.size)));
        }
        _ => out.push((tag.path.clone(), Shown::Sized(prepared.size))),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(w, h).write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-sent-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_file_is_asked_for_once_and_shows_loading_until_it_comes() {
        let mut sent = Sent::default();
        assert!(matches!(sent.look("/u/a.png", Want::Picture), Shown::Loading));
        assert!(matches!(sent.look("/u/a.png", Want::Picture), Shown::Loading));
        assert!(matches!(sent.look("/u/b.pdf", Want::Size), Shown::Loading));
        assert_eq!(sent.take_wanted(), [("/u/a.png".to_owned(), Want::Picture), ("/u/b.pdf".to_owned(), Want::Size)]);
        assert!(sent.take_wanted().is_empty());
        sent.put("/u/b.pdf", Shown::Sized(12));
        assert!(matches!(sent.look("/u/b.pdf", Want::Size), Shown::Sized(12)));
        sent.put("/u/a.png", Shown::Gone);
        sent.retry_gone();
        assert!(sent.get("/u/a.png").is_none(), "asked again once back on the line");
        let generation = sent.generation;
        sent.clear();
        assert!(sent.get("/u/b.pdf").is_none());
        assert_eq!(sent.generation, generation + 1);
    }

    #[test]
    fn what_was_just_sent_isnt_replaced_by_a_lesser_answer() {
        let mut sent = Sent::default();
        let picture = picture(&png(10, 10), false).unwrap();
        sent.seed("/u/a.png", Shown::Picture(picture));
        sent.seed("/u/a.png", Shown::Sized(3));
        assert!(matches!(sent.get("/u/a.png"), Some(Shown::Picture(_))));
    }

    #[test]
    fn whole_pictures_from_afar_stay_within_their_budget() {
        let mut sent = Sent::default();
        let big = |n: usize| Picture { thumb: Arc::new(Image::from_bytes(ImageFormat::Png, vec![])), dims: (1, 1), full: Some(Arc::new(Image::from_bytes(ImageFormat::Png, vec![0; n]))), size: n as u64, at: Instant::now() };
        for i in 0..5 {
            sent.put(&format!("/u/{i}.png"), Shown::Picture(big(20 << 20)));
        }
        assert!(sent.whole_bytes() <= FULL_BUDGET);
        let whole = |p: &str| matches!(sent.get(p), Some(Shown::Picture(Picture { full: Some(_), .. })));
        assert!(!whole("/u/0.png") && whole("/u/4.png"), "the oldest are let go, the thumbnails stay");
        assert!(matches!(sent.get("/u/0.png"), Some(Shown::Picture(_))));
        // Put again: counted once.
        sent.put("/u/4.png", Shown::Picture(big(20 << 20)));
        assert!(sent.whole_bytes() <= FULL_BUDGET);
    }

    #[test]
    fn a_message_shows_its_words_and_at_most_twenty_attachments() {
        let text = format!("봐 줘 {}", (0..23).map(|i| format!("[image: /u/{i}.png]")).collect::<Vec<_>>().join(" "));
        let p = parts(&text);
        assert_eq!(p.text, "봐 줘");
        assert_eq!((p.tags.len(), p.more), (MAX_SHOWN, 3));
        assert_eq!(parts("plain").tags.len(), 0);
    }

    #[test]
    fn where_a_message_is_named_its_attachments_read_as_kind_and_name() {
        assert_eq!(summary("pair 가 잘 안돼 [image: /u/1792073093606-paste.png]"), "pair 가 잘 안돼 [이미지] paste.png");
        assert_eq!(summary("[pdf: /u/1792073093606-spec.pdf · 2 pages]"), "[PDF] spec.pdf");
        assert_eq!(summary("그냥 글"), "그냥 글");
    }

    #[test]
    fn each_kind_wants_its_picture_or_its_size() {
        let p = parts("[image: /u/a.png] [video: /u/v.mov · 0:20 · sheet: /u/s.jpg · frames: /u/f1.jpg@0:05] [video: /u/bare.mov] [pdf: /u/d.pdf · 2 pages] [audio: /u/m.m4a · 0:31]");
        let wants: Vec<_> = p.tags.iter().map(want_of).collect();
        assert_eq!(wants, [Some(("/u/a.png", Want::Picture)), Some(("/u/f1.jpg", Want::Picture)), None, Some(("/u/d.pdf", Want::Size)), Some(("/u/m.m4a", Want::Size))]);
        assert_eq!(gallery_of(&p.tags), ["/u/a.png", "/u/s.jpg"]);
    }

    #[test]
    fn the_lightbox_goes_round_the_messages_pictures() {
        let mut g = Gallery::open(vec!["/a".into(), "/b".into(), "/c".into()], "/b").unwrap();
        g.step(true);
        assert_eq!(g.path(), "/c");
        g.step(true);
        assert_eq!(g.path(), "/a");
        g.step(false);
        assert_eq!(g.path(), "/c");
        assert!(Gallery::open(vec!["/a".into()], "/z").is_none());
    }

    #[test]
    fn one_picture_keeps_its_shape_within_its_bounds() {
        // A 2x screenshot, 1568×980: shown as 384×240 points, no taller.
        assert_eq!(single_size(Some((1568, 980)), 600.), (384., 240.));
        // Tall and narrow: 240 high.
        assert_eq!(single_size(Some((400, 1600)), 600.), (60., 240.));
        // Wide: no wider than WIDE, or the room there is.
        assert_eq!(single_size(Some((4000, 400)), 600.), (420., 42.));
        assert_eq!(single_size(Some((4000, 400)), 300.), (300., 30.));
        // Small: its own size, never enlarged.
        assert_eq!(single_size(Some((200, 100)), 600.), (100., 50.));
        assert_eq!(single_size(None, 600.), UNKNOWN);
        assert_eq!(single_size(Some((0, 5)), 600.), UNKNOWN);
    }

    #[test]
    fn a_file_on_this_mac_is_read_as_a_thumbnail_or_its_size_or_gone() {
        let dir = dir("load");
        let shot = dir.join("1792073093606-paste.png");
        std::fs::write(&shot, png(2000, 1000)).unwrap();
        std::fs::write(dir.join("a.pdf"), b"%PDF-1.4 xx").unwrap();
        let path = shot.to_string_lossy().into_owned();
        match load(&Where::Local, &path, Want::Picture) {
            Shown::Picture(p) => {
                assert_eq!(p.dims, (2000, 1000));
                assert!(p.full.is_none(), "on this Mac the file itself is shown big");
                let thumb = image::load_from_memory(&p.thumb.bytes).unwrap();
                assert!(thumb.width() <= THUMB_W && thumb.height() <= THUMB_H);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(load(&Where::Local, &dir.join("a.pdf").to_string_lossy(), Want::Size), Shown::Sized(11)));
        assert!(matches!(load(&Where::Local, &dir.join("gone.png").to_string_lossy(), Want::Picture), Shown::Gone));
        assert!(matches!(load(&Where::Local, &dir.join("a.pdf").to_string_lossy(), Want::Picture), Shown::Gone), "not a picture");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn what_was_just_sent_is_kept_by_the_paths_its_tag_names() {
        let prepared = crate::attach::prepare(crate::attach::Source::Bytes { name: "paste.png".into(), bytes: png(40, 20) }).unwrap();
        let ready = Ready { path: "/up/1-paste.png".into(), tag: "[image: /up/1-paste.png]".into() };
        let kept = just_sent(&prepared, &ready, true);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].0, "/up/1-paste.png");
        assert!(matches!(&kept[0].1, Shown::Picture(p) if p.dims == (40, 20) && p.full.is_some()));

        let dir = dir("sent");
        std::fs::write(dir.join("spec.pdf"), b"%PDF").unwrap();
        let file = Prepared { kind: crate::attach::Kind::Pdf, name: "spec.pdf".into(), size: 4, thumb: None, meta: Default::default(), main: Payload::File { name: "spec.pdf".into(), path: dir.join("spec.pdf") }, frames: vec![], sheet: None };
        let kept = just_sent(&file, &Ready { path: "/up/2-spec.pdf".into(), tag: "[pdf: /up/2-spec.pdf · 1 page]".into() }, false);
        assert!(matches!(kept.as_slice(), [(p, Shown::Sized(4))] if p == "/up/2-spec.pdf"));

        let video = Prepared {
            kind: crate::attach::Kind::Video,
            name: "a.mov".into(),
            size: 99,
            thumb: None,
            meta: Default::default(),
            main: Payload::File { name: "a.mov".into(), path: dir.join("a.mov") },
            frames: vec![(5., Payload::Bytes { name: "f.jpg".into(), bytes: png(8, 8) })],
            sheet: None,
        };
        let kept = just_sent(&video, &Ready { path: "/up/a.mov".into(), tag: "[video: /up/a.mov · 0:20 · frames: /up/f.jpg@0:05]".into() }, false);
        assert_eq!(kept.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(), ["/up/f.jpg", "/up/a.mov"]);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
