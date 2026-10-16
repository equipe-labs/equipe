//! A member's pictures — a visual decision's options, what a hand-in shows —
//! as the window has them: asked of the daemon (`read_image`), on this Mac
//! or another alike, so they come only when the daemon says the squad shows
//! them; a thumbnail first, and the picture whole once it is opened big,
//! the whole ones kept within a budget. And how they lay out: the options'
//! grid, a hand-in's gallery, the strip under a news line, the big view's
//! ←/→. No UI types beyond the image, so the rules are tested without a window.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use equipe_core::protocol::TicketId;
use equipe_core::protocol::decide::Question;
use equipe_core::protocol::rpc::ImageRead;
use gpui_kit::{Image, ImageFormat};

/// A thumbnail's longer side, in pixels: a tile is at most [`TILE_MAX`]
/// points wide, drawn at 2x.
pub const THUMB: u32 = 640;
/// A picture opened big: no more than this many pixels on its longer side.
pub const FULL: u32 = 2560;
/// A tile's picture box: this high for its width (4:3), whatever the picture.
pub const RATIO: f32 = 0.75;
/// A tile is never narrower than this: past it, one column fewer.
pub const TILE_MIN: f32 = 176.;
/// …nor wider than this: past it, the grid stops filling the row.
pub const TILE_MAX: f32 = 300.;
/// The space between tiles.
pub const GAP: f32 = 10.;
/// The most columns a grid has.
const COLUMNS_MAX: usize = 4;
/// A strip's tiles (a news line, the hover card): this wide, 4:3.
pub const STRIP: f32 = 64.;
/// How many a strip shows before "+n".
pub const STRIP_MAX: usize = 4;
/// Pictures kept whole, at most this many bytes all told.
const WHOLE_BUDGET: usize = 96 << 20;

/// How big a picture is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Size {
    Thumb,
    Full,
}

impl Size {
    pub fn side(self) -> u32 {
        match self {
            Size::Thumb => THUMB,
            Size::Full => FULL,
        }
    }
}

/// What is known of one picture at one size.
#[derive(Debug, Clone)]
pub enum Look {
    /// Asked for, not here yet.
    Loading,
    Ready(Shot),
    /// Not there any more, or not a picture the squad shows.
    Gone,
}

#[derive(Debug, Clone)]
pub struct Shot {
    pub image: Arc<Image>,
    /// Its size in pixels, as the file is (an SVG's isn't known).
    pub dims: Option<(u32, u32)>,
    pub bytes: usize,
    /// When it came: it fades in for a moment after.
    pub at: Instant,
}

/// Every picture the window has asked about, by its path on the daemon's
/// machine and size; and how wide each grid was last drawn.
#[derive(Debug, Default)]
pub struct Visuals {
    shown: HashMap<(String, Size), Look>,
    wanted: Vec<(String, Size)>,
    /// A load of what is wanted is on its way to being started.
    pub scheduled: bool,
    /// Counts up when the machine or the squad changes: loads started before don't land.
    pub generation: u64,
    whole: VecDeque<String>,
    whole_bytes: usize,
    widths: HashMap<u64, f32>,
}

impl Visuals {
    /// What `path` shows at `size` now; the first time, it is asked for.
    pub fn look(&mut self, path: &str, size: Size) -> Look {
        let key = (path.to_owned(), size);
        if let Some(look) = self.shown.get(&key) {
            return look.clone();
        }
        self.shown.insert(key, Look::Loading);
        self.wanted.push((path.to_owned(), size));
        Look::Loading
    }

    /// The best of `path` there is: whole when it came, else its thumbnail.
    pub fn best(&self, path: &str) -> Option<Shot> {
        [Size::Full, Size::Thumb].into_iter().find_map(|size| match self.shown.get(&(path.to_owned(), size)) {
            Some(Look::Ready(shot)) => Some(shot.clone()),
            _ => None,
        })
    }

    /// Whether `path` turned out not to be there.
    pub fn is_gone(&self, path: &str) -> bool {
        matches!(self.shown.get(&(path.to_owned(), Size::Thumb)), Some(Look::Gone))
    }

    /// Whether anything waits to be asked for.
    pub fn has_wanted(&self) -> bool {
        !self.wanted.is_empty()
    }

    /// What is to be loaded, taken.
    pub fn take_wanted(&mut self) -> Vec<(String, Size)> {
        self.scheduled = false;
        std::mem::take(&mut self.wanted)
    }

    /// What `path` shows at `size` from now on. Whole pictures past the
    /// budget are let go, oldest first (asked again when opened again).
    pub fn put(&mut self, path: &str, size: Size, look: Look) {
        if size == Size::Full
            && let Look::Ready(shot) = &look
        {
            self.whole.retain(|p| p != path);
            self.whole.push_back(path.to_owned());
            self.whole_bytes += shot.bytes;
        }
        self.shown.insert((path.to_owned(), size), look);
        while self.whole_bytes > WHOLE_BUDGET && self.whole.len() > 1 {
            let Some(oldest) = self.whole.pop_front() else { break };
            if let Some(Look::Ready(shot)) = self.shown.remove(&(oldest, Size::Full)) {
                self.whole_bytes = self.whole_bytes.saturating_sub(shot.bytes);
            }
        }
    }

    /// Back on the line: what couldn't be had is asked for again.
    pub fn retry_gone(&mut self) {
        self.shown.retain(|_, look| !matches!(look, Look::Gone));
    }

    /// Another machine or squad: nothing known holds.
    pub fn clear(&mut self) {
        *self = Self { generation: self.generation + 1, ..Self::default() };
    }

    /// How wide grid `key` was last drawn.
    pub fn width(&self, key: u64) -> Option<f32> {
        self.widths.get(&key).copied()
    }

    /// Grid `key` was drawn `width` wide: whether that differs from before.
    pub fn set_width(&mut self, key: u64, width: f32) -> bool {
        let before = self.widths.insert(key, width);
        before.is_none_or(|w| (w - width).abs() > 0.5)
    }

    #[cfg(test)]
    fn whole_bytes(&self) -> usize {
        self.whole_bytes
    }
}

/// The picture the daemon sent, ready to draw; `None` for bytes that aren't one.
pub fn shot(read: &ImageRead) -> Option<Shot> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD.decode(&read.data).ok()?;
    let format = match read.mime.as_str() {
        "image/png" => ImageFormat::Png,
        "image/jpeg" => ImageFormat::Jpeg,
        "image/gif" => ImageFormat::Gif,
        "image/webp" => ImageFormat::Webp,
        "image/svg+xml" => ImageFormat::Svg,
        _ => return None,
    };
    let dims = read.width.zip(read.height);
    Some(Shot { bytes: bytes.len(), image: Arc::new(Image::from_bytes(format, bytes)), dims, at: Instant::now() })
}

// ── laying out ──

/// How many columns `n` tiles take in `width`: as many as fit at
/// [`TILE_MIN`] each, never more than there are tiles — one, when narrow.
pub fn columns(width: f32, n: usize) -> usize {
    let fit = ((width + GAP) / (TILE_MIN + GAP)).floor().max(1.) as usize;
    fit.min(n.max(1)).min(COLUMNS_MAX)
}

/// How wide each of `columns` tiles is in `width`: the row shared out, but
/// no wider than [`TILE_MAX`].
pub fn tile_width(width: f32, columns: usize) -> f32 {
    let columns = columns.max(1) as f32;
    ((width - GAP * (columns - 1.)) / columns).clamp(1., TILE_MAX).floor()
}

/// A picture of `dims` drawn whole inside `room` (contain): its size.
pub fn contain(dims: (u32, u32), room: (f32, f32)) -> (f32, f32) {
    let (w, h) = (dims.0.max(1) as f32, dims.1.max(1) as f32);
    let scale = (room.0 / w).min(room.1 / h);
    ((w * scale).floor(), (h * scale).floor())
}

/// The words members mark the option they recommend with, in the languages
/// the person may speak — "(Recommended)", "(추천)".
const RECOMMENDED: [&str; 14] = [
    "recommended", "추천", "권장", "おすすめ", "推奨", "推荐", "推薦", "recomendado", "recomendada", "recommandé", "recommandée", "empfohlen", "empfehlung", "best",
];

/// An option's label without its "(Recommended)", and whether it had one.
pub fn recommended(label: &str) -> (String, bool) {
    let trimmed = label.trim_end();
    for (open, close) in [('(', ')'), ('（', '）')] {
        if let Some(inner) = trimmed.strip_suffix(close)
            && let Some(at) = inner.rfind(open)
        {
            let word = inner[at + open.len_utf8()..].trim().to_lowercase();
            if RECOMMENDED.contains(&word.as_str()) {
                return (inner[..at].trim_end().to_owned(), true);
            }
        }
    }
    (label.to_owned(), false)
}

// ── shown big ──

/// What the big view goes through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Owner {
    /// A question's options: set and question; a pick from here is the person's.
    Decision { set: u64, step: usize },
    /// A ticket's hand-in.
    Ticket(TicketId),
}

/// One picture of the big view, and what it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub path: String,
    /// The option it shows (a decision's), by index.
    pub choice: Option<usize>,
    /// What it is called: the option's label, or the file's name.
    pub label: String,
}

/// Pictures shown big over the window, one at a time; ←/→ go through them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Viewer {
    pub owner: Owner,
    pub items: Vec<Item>,
    pub at: usize,
}

/// A file's name, from its path.
pub fn name_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

impl Viewer {
    /// A question's pictures — its own, then each option's — open at `path`.
    pub fn of_question(set: u64, step: usize, q: &Question, path: &str) -> Option<Self> {
        let own = q.images.iter().map(|p| Item { path: p.clone(), choice: None, label: name_of(p).to_owned() });
        let options = q.options.iter().enumerate().flat_map(|(n, c)| c.images.iter().map(move |p| Item { path: p.clone(), choice: Some(n), label: recommended(&c.label).0 }));
        let items: Vec<Item> = own.chain(options).collect();
        let at = items.iter().position(|i| i.path == path)?;
        Some(Self { owner: Owner::Decision { set, step }, items, at })
    }

    /// A hand-in's pictures, open at `path`.
    pub fn of_ticket(id: TicketId, images: &[String], path: &str) -> Option<Self> {
        let items: Vec<Item> = images.iter().map(|p| Item { path: p.clone(), choice: None, label: name_of(p).to_owned() }).collect();
        let at = items.iter().position(|i| i.path == path)?;
        Some(Self { owner: Owner::Ticket(id), items, at })
    }

    pub fn item(&self) -> &Item {
        &self.items[self.at.min(self.items.len().saturating_sub(1))]
    }

    pub fn step(&mut self, forward: bool) {
        let n = self.items.len().max(1);
        self.at = if forward { (self.at + 1) % n } else { (self.at + n - 1) % n };
    }

    /// To option `n`'s first picture, if it has one.
    pub fn go_to_choice(&mut self, n: usize) -> bool {
        match self.items.iter().position(|i| i.choice == Some(n)) {
            Some(at) => {
                self.at = at;
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::decide::Choice;

    fn shot(bytes: usize) -> Look {
        Look::Ready(Shot { image: Arc::new(Image::from_bytes(ImageFormat::Png, vec![0; 4])), dims: Some((4, 4)), bytes, at: Instant::now() })
    }

    #[test]
    fn a_picture_is_asked_for_once_per_size_and_the_best_there_is_shows() {
        let mut v = Visuals::default();
        assert!(matches!(v.look("/w/a.png", Size::Thumb), Look::Loading));
        assert!(matches!(v.look("/w/a.png", Size::Thumb), Look::Loading));
        assert_eq!(v.take_wanted(), [("/w/a.png".to_owned(), Size::Thumb)], "once");
        assert!(v.best("/w/a.png").is_none());
        v.put("/w/a.png", Size::Thumb, shot(10));
        assert_eq!(v.best("/w/a.png").unwrap().bytes, 10);
        v.put("/w/a.png", Size::Full, shot(1000));
        assert_eq!(v.best("/w/a.png").unwrap().bytes, 1000, "whole, once it came");
        v.put("/w/gone.png", Size::Thumb, Look::Gone);
        assert!(v.is_gone("/w/gone.png"));
        v.retry_gone();
        assert!(matches!(v.look("/w/gone.png", Size::Thumb), Look::Loading), "asked again once back on the line");
        let generation = v.generation;
        v.clear();
        assert!(v.best("/w/a.png").is_none() && v.generation == generation + 1);
    }

    #[test]
    fn whole_pictures_past_the_budget_go_oldest_first_and_their_thumbnails_stay() {
        let mut v = Visuals::default();
        for n in 0..4 {
            let path = format!("/w/{n}.png");
            v.put(&path, Size::Thumb, shot(1));
            v.put(&path, Size::Full, shot(40 << 20));
        }
        assert!(v.whole_bytes() <= WHOLE_BUDGET);
        assert_eq!(v.best("/w/0.png").unwrap().bytes, 1, "let go whole; the thumbnail stays");
        assert_eq!(v.best("/w/3.png").unwrap().bytes, 40 << 20);
    }

    #[test]
    fn a_grid_fills_its_row_and_wraps_to_one_column_when_narrow() {
        assert_eq!(columns(640., 3), 3);
        assert_eq!(columns(640., 2), 2, "never more than there are");
        assert_eq!(columns(400., 3), 2);
        assert_eq!(columns(300., 3), 1, "narrow: one column");
        assert_eq!(columns(2000., 9), 4, "at most four");
        assert_eq!(columns(0., 0), 1);
        let (w3, w1) = (tile_width(640., 3), tile_width(300., 1));
        assert!((3. * w3 + 2. * GAP) <= 640. && w3 >= TILE_MIN, "{w3}");
        assert_eq!(w1, TILE_MAX, "one wide tile stops at the most a tile is");
        assert_eq!(tile_width(260., 1), 260.);
        // Every tile of a grid is the same 4:3 box.
        assert_eq!((w3 * RATIO).round(), (w3 * 0.75).round());
    }

    #[test]
    fn a_picture_shown_whole_keeps_its_shape_inside_its_box() {
        assert_eq!(contain((2000, 1000), (400., 300.)), (400., 200.));
        assert_eq!(contain((500, 1000), (400., 300.)), (150., 300.));
        assert_eq!(contain((0, 0), (40., 30.)), (30., 30.), "an unknown size: square");
    }

    #[test]
    fn the_recommended_option_reads_without_its_mark_in_any_language() {
        assert_eq!(recommended("e. + 파란 원 점 (추천)"), ("e. + 파란 원 점".to_owned(), true));
        assert_eq!(recommended("Postgres (Recommended)"), ("Postgres".to_owned(), true));
        assert_eq!(recommended("青い点（推荐）"), ("青い点".to_owned(), true));
        assert_eq!(recommended("Bleu (Recommandé) "), ("Bleu".to_owned(), true));
        assert_eq!(recommended("e. (회색 점)"), ("e. (회색 점)".to_owned(), false), "brackets of its own stay");
        assert_eq!(recommended("SQLite"), ("SQLite".to_owned(), false));
    }

    #[test]
    fn the_big_view_goes_through_a_questions_pictures_and_finds_each_option() {
        let option = |label: &str, images: &[&str]| Choice { label: label.into(), images: images.iter().map(|i| (*i).to_owned()).collect(), ..Choice::default() };
        let q = Question {
            question: "로고?".into(),
            images: vec!["/w/now.png".into()],
            options: vec![option("파란 점 (추천)", &["/w/a.png", "/w/a-dark.png"]), option("회색 점", &["/w/b.png"]), option("직접", &[])],
            ..Question::default()
        };
        let mut v = Viewer::of_question(7, 0, &q, "/w/b.png").unwrap();
        assert_eq!(v.items.len(), 4);
        assert_eq!((v.at, v.item().choice, v.item().label.as_str()), (3, Some(1), "회색 점"));
        v.step(true);
        assert_eq!((v.item().path.as_str(), v.item().choice), ("/w/now.png", None), "round to the first");
        v.step(false);
        v.step(false);
        assert_eq!(v.item().label, "파란 점", "the option's label, without its mark");
        assert!(v.go_to_choice(0) && v.item().path == "/w/a.png");
        assert!(!v.go_to_choice(2), "an option without pictures");
        assert!(Viewer::of_question(7, 0, &q, "/w/elsewhere.png").is_none());
        let t = Viewer::of_ticket(144, &["/w/brand/a.png".into(), "/w/brand/b.svg".into()], "/w/brand/b.svg").unwrap();
        assert_eq!((t.owner.clone(), t.at, t.item().label.as_str()), (Owner::Ticket(144), 1, "b.svg"));
    }
}
