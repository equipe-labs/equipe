//! A member's pictures — a visual decision's options, what a hand-in shows —
//! as the window has them: asked of the daemon (`read_image`), on this Mac
//! or another alike, so they come only when the daemon says the squad shows
//! them; a thumbnail first, and the picture whole once it is opened big,
//! the whole ones kept within a budget. And how they lay out: the options'
//! grid, a hand-in's gallery, the strip under a news line, the big view's
//! ←/→. No UI types beyond the image, so the rules are tested without a window.
//!
//! A reply's pictures come the same way: each image it embeds at the
//! column's width (`Size::Inline`), and the pictures of a folder it names
//! (`list_images`), listed once.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use equipe_core::protocol::TicketId;
use equipe_core::protocol::decide::Question;
use equipe_core::protocol::rpc::{ImageList, ImageRead, ImageWhy};
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
/// A picture in a reply: its longer side in pixels, for the column at 2x.
pub const INLINE: u32 = 1280;
/// A picture in a reply is never taller than this, in points…
pub const INLINE_HIGH: f32 = 420.;
/// …and, while its size isn't known, this wide, 4:3.
pub const INLINE_GUESS: f32 = 480.;
/// A folder's strip under a reply: each tile this wide, 4:3.
pub const FOLDER_TILE: f32 = 88.;
/// The key the conversation's column width is kept under.
pub const COLUMN_KEY: u64 = u64::MAX;

/// How big a picture is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Size {
    Thumb,
    /// As wide as the conversation's column, at 2x.
    Inline,
    Full,
}

impl Size {
    pub fn side(self) -> u32 {
        match self {
            Size::Thumb => THUMB,
            Size::Inline => INLINE,
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
    /// Not there any more, too big, or not a picture the squad shows.
    Gone(ImageWhy),
}

/// What is known of a folder a reply names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Listing {
    Loading,
    Ready(ImageList),
    /// Not a folder of pictures the squad shows (or a daemon too old to say).
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
    folders: HashMap<String, Listing>,
    wanted_folders: Vec<String>,
    /// The conversation's rows each picture or folder is drawn in: measured
    /// again when it comes, as its size is known then.
    rows: HashMap<String, HashSet<u64>>,
    /// Hand-in galleries whose assets the person unfolded.
    pub unfolded: HashSet<u64>,
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

    /// What `path` shows at `size` now, without asking for it.
    pub fn known(&self, path: &str, size: Size) -> Option<&Look> {
        self.shown.get(&(path.to_owned(), size))
    }

    /// The best of `path` there is: whole when it came, else the reply's, else its thumbnail.
    pub fn best(&self, path: &str) -> Option<Shot> {
        [Size::Full, Size::Inline, Size::Thumb].into_iter().find_map(|size| match self.shown.get(&(path.to_owned(), size)) {
            Some(Look::Ready(shot)) => Some(shot.clone()),
            _ => None,
        })
    }

    /// Whether `path` turned out not to be there.
    pub fn is_gone(&self, path: &str) -> bool {
        matches!(self.shown.get(&(path.to_owned(), Size::Thumb)), Some(Look::Gone(_)))
    }

    /// What folder `folder` holds; the first time, it is asked for.
    pub fn listing(&mut self, folder: &str) -> Listing {
        if let Some(listing) = self.folders.get(folder) {
            return listing.clone();
        }
        self.folders.insert(folder.to_owned(), Listing::Loading);
        self.wanted_folders.push(folder.to_owned());
        Listing::Loading
    }

    pub fn put_listing(&mut self, folder: &str, listing: Listing) {
        self.folders.insert(folder.to_owned(), listing);
    }

    /// Whether anything waits to be asked for.
    pub fn has_wanted(&self) -> bool {
        !self.wanted.is_empty() || !self.wanted_folders.is_empty()
    }

    /// What is to be loaded, taken.
    pub fn take_wanted(&mut self) -> Vec<(String, Size)> {
        self.scheduled = false;
        std::mem::take(&mut self.wanted)
    }

    /// The folders to be listed, taken.
    pub fn take_wanted_folders(&mut self) -> Vec<String> {
        std::mem::take(&mut self.wanted_folders)
    }

    /// `path` (a picture, or a folder) is drawn in conversation row `row`.
    pub fn in_row(&mut self, path: &str, row: u64) {
        if let Some(rows) = self.rows.get_mut(path) {
            rows.insert(row);
            return;
        }
        if self.rows.len() > 2048 {
            self.rows.clear();
        }
        self.rows.insert(path.to_owned(), HashSet::from([row]));
    }

    /// The rows `path` is drawn in.
    pub fn rows_of(&self, path: &str) -> Vec<u64> {
        self.rows.get(path).map(|r| r.iter().copied().collect()).unwrap_or_default()
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
        self.shown.retain(|_, look| !matches!(look, Look::Gone(_)));
        self.folders.retain(|_, listing| *listing != Listing::Gone);
    }

    /// The person asked again for `paths`: they are asked for once more.
    pub fn retry(&mut self, paths: &[String]) {
        self.shown.retain(|(path, _), look| !(matches!(look, Look::Gone(_)) && paths.contains(path)));
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

/// How a grid of tiles lays out: how many each row has, top to bottom, and
/// how wide every tile is.
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    pub rows: Vec<usize>,
    pub tile: f32,
}

/// `n` tiles in `width`: as many a row as fit (at most four), the rows
/// balanced so the last isn't left with one while another has room to give —
/// 4 → 2+2, 5 → 3+2, 7 → 3+2+2. Every tile is one width: the row's share
/// when all fit in one, else what the rows that fit allow — so wrapping
/// never makes a tile bigger than its neighbours in a wider row would be.
pub fn grid(width: f32, n: usize) -> Grid {
    let fit = columns(width, n);
    Grid { rows: balance(n, fit), tile: tile_width(width, fit) }
}

/// [`grid`], with every tile short enough that a whole row — its picture and
/// its caption — fits `room` points high: where the card is short, smaller
/// tiles (more to a row) rather than captions cut off under the box below.
/// Never narrower than [`TILE_SHRUNK`].
pub fn grid_within(width: f32, n: usize, described: bool, room: f32) -> Grid {
    let grid = grid(width, n);
    if tile_height(grid.tile, described) <= room {
        return grid;
    }
    let fixed = tile_height(0., described);
    let tile = ((room - fixed) / RATIO).floor().max(TILE_SHRUNK).min(grid.tile);
    let per = per_row(width, tile).min(n.max(1));
    Grid { rows: balance(n, per), tile }
}

/// The narrowest a tile gets to fit a short card.
pub const TILE_SHRUNK: f32 = 120.;

/// `n` tiles in rows of at most `per`, as even as they come: the fuller rows first.
pub fn balance(n: usize, per: usize) -> Vec<usize> {
    if n == 0 {
        return Vec::new();
    }
    let count = n.div_ceil(per.max(1));
    let (base, extra) = (n / count, n % count);
    (0..count).map(|r| base + usize::from(r < extra)).collect()
}

/// How many tiles `tile` wide fit a row `width` wide (one at least).
pub fn per_row(width: f32, tile: f32) -> usize {
    (((width + GAP) / (tile + GAP)).floor().max(1.) as usize).min(COLUMNS_MAX)
}

impl Grid {
    /// Which tiles each row holds, by index.
    pub fn spans(&self) -> Vec<std::ops::Range<usize>> {
        let mut at = 0;
        self.rows
            .iter()
            .map(|&k| {
                at += k;
                at - k..at
            })
            .collect()
    }

    /// Tile `ix`'s box, `(x, y, w, h)` from the grid's top left, `h` tall —
    /// in any state: picked, under the pointer or the keys, it neither moves
    /// nor grows ([`frame`]).
    pub fn rect(&self, ix: usize, h: f32, state: TileState) -> Option<(f32, f32, f32, f32)> {
        let row = self.spans().iter().position(|s| s.contains(&ix))?;
        let col = ix - self.spans()[row].start;
        let f = frame(state);
        let (x, y) = (col as f32 * (self.tile + GAP), row as f32 * (h + GAP) - f.lift);
        Some((x - f.outset, y - f.outset, self.tile + 2. * f.outset, h + 2. * f.outset))
    }

    /// How tall the grid is with tiles `h` tall.
    pub fn height(&self, h: f32) -> f32 {
        let n = self.rows.len() as f32;
        (n * h + (n - 1.).max(0.) * GAP).max(0.)
    }
}

/// A tile's state, as the person sees it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TileState {
    pub picked: bool,
    /// Where the keys are.
    pub cursor: bool,
    /// Under the pointer, 0 to 1 as it eases in.
    pub hover: f32,
}

/// What a tile draws in a state, as far as its box goes: its border, the
/// ring drawn inside it, how far it moves up and how far past its box it
/// reaches. Only paint tells states apart — the border is always one point,
/// the ring always inside, nothing lifts or spreads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub border: f32,
    pub ring: f32,
    pub lift: f32,
    pub outset: f32,
}

/// The ring of a picked tile, inside its border: together, three points.
pub const RING: f32 = 2.;

pub fn frame(state: TileState) -> Frame {
    Frame { border: 1., ring: if state.picked { RING } else { 0. }, lift: 0., outset: 0. }
}

/// A tile's caption: its label on one line, and — when any option of the
/// question has one — two lines of what it means, room kept for them on
/// every tile so a grid's tiles are one height.
pub fn caption_height(described: bool) -> f32 {
    const PAD: f32 = 8. + 9.;
    const LINES: f32 = 2. + 2. * 17.;
    PAD + CAPTION_TITLE + if described { LINES } else { 0. }
}

/// A decision's tile `tile` wide, all told: its picture's 4:3 box, its
/// caption and the hairline over it, and its border — the same in every state.
pub fn tile_height(tile: f32, described: bool) -> f32 {
    (tile * RATIO).floor() + caption_height(described) + 1. + 2. * frame(TileState::default()).border
}

/// A caption's line for the label, as high as its number key.
pub const CAPTION_TITLE: f32 = 20.;

/// A question's own picture when its options have none: big over them, as
/// wide as the card allows up to [`LEAD_WIDE`], 4:3 up to [`LEAD_HIGH`].
pub fn lead_box(width: f32) -> (f32, f32) {
    let w = width.clamp(1., LEAD_WIDE).floor();
    (w, (w * RATIO).min(LEAD_HIGH).floor())
}

/// How wide a question's own picture is, at most, when shown big.
pub const LEAD_WIDE: f32 = 480.;
/// …and how high.
pub const LEAD_HIGH: f32 = 300.;

/// A picture of `dims` drawn whole inside `room` (contain): its size.
pub fn contain(dims: (u32, u32), room: (f32, f32)) -> (f32, f32) {
    let (w, h) = (dims.0.max(1) as f32, dims.1.max(1) as f32);
    let scale = (room.0 / w).min(room.1 / h);
    ((w * scale).floor(), (h * scale).floor())
}

/// The box a picture in a reply takes in a column `column` wide: its own
/// shape, never wider than the column nor taller than [`INLINE_HIGH`], nor
/// bigger than it is (a pixel a point); 4:3 while its size isn't known.
pub fn inline_box(dims: Option<(u32, u32)>, column: f32) -> (f32, f32) {
    let column = column.max(80.);
    match dims {
        Some((w, h)) => contain((w, h), (column.min(w.max(1) as f32), INLINE_HIGH.min(h.max(1) as f32))),
        None => {
            let w = column.min(INLINE_GUESS).floor();
            (w, (w * RATIO).min(INLINE_HIGH).floor())
        }
    }
}

/// A hand-in's gallery: its work, and its source assets folded under it —
/// each asset once, whatever sizes it comes in (`visuals::arrange`).
pub fn arrange(images: &[String]) -> (Vec<String>, Vec<String>) {
    let arranged = equipe_core::visuals::arrange(images);
    (arranged.work, arranged.assets)
}

/// How the pictures of a gallery that couldn't be had read, when all of them
/// failed: one line, with the reason most of them share.
pub fn all_failed(looks: &[Option<&Look>]) -> Option<ImageWhy> {
    if looks.is_empty() {
        return None;
    }
    let mut whys: Vec<ImageWhy> = Vec::new();
    for look in looks {
        match look {
            Some(Look::Gone(why)) => whys.push(*why),
            _ => return None,
        }
    }
    let count = |w: ImageWhy| whys.iter().filter(|x| **x == w).count();
    [ImageWhy::Missing, ImageWhy::TooBig, ImageWhy::Outside, ImageWhy::Unreadable].into_iter().max_by_key(|w| count(*w))
}

/// The words members mark the option they recommend with, in the languages
/// the person may speak — "(Recommended)", "(추천)".
fn recommended_words() -> &'static [&'static str] {
    crate::i18n::every("match.recommended")
}

/// An option's label without its "(Recommended)", and whether it had one.
pub fn recommended(label: &str) -> (String, bool) {
    let trimmed = label.trim_end();
    for (open, close) in [('(', ')'), ('（', '）')] {
        if let Some(inner) = trimmed.strip_suffix(close)
            && let Some(at) = inner.rfind(open)
        {
            let word = inner[at + open.len_utf8()..].trim().to_lowercase();
            if recommended_words().contains(&word.as_str()) {
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
    /// The pictures of one row of the conversation: those it embeds, then its folders'.
    Reply(u64),
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

/// How far the big view is zoomed in on its picture, and where: 1 is the
/// picture whole in the window; past it, `pan` moves it (px, from centred).
/// Kept for one picture: another shown starts whole again.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Zoom {
    pub path: String,
    pub scale: f32,
    pub pan: (f32, f32),
    pub drag: Option<Drag>,
}

/// A drag under way: where it was pressed, the pan then, and whether it moved.
pub type Drag = ((f32, f32), (f32, f32), bool);

/// The steps `+` and `−` go through.
pub const ZOOMS: [f32; 8] = [1., 1.5, 2., 3., 4., 6., 8., 12.];

/// Under this when a pinch ends, the picture goes whole.
pub const PINCH_SETTLE: f32 = 1.03;

impl Zoom {
    /// Its scale for `path`: 1 for a picture it wasn't zoomed on.
    pub fn of(&self, path: &str) -> (f32, (f32, f32)) {
        if self.path == path && self.scale > 0. { (self.scale, self.pan) } else { (1., (0., 0.)) }
    }

    /// To `scale` on `path`, keeping the point `at` (px from the view's
    /// centre) where it is — the pointer's, or the centre's.
    pub fn set(&mut self, path: &str, scale: f32, at: (f32, f32)) {
        let (old, pan) = self.of(path);
        let scale = scale.clamp(ZOOMS[0], ZOOMS[ZOOMS.len() - 1]);
        let k = scale / old;
        self.path = path.to_owned();
        self.scale = scale;
        self.pan = if scale <= 1. { (0., 0.) } else { (at.0 - (at.0 - pan.0) * k, at.1 - (at.1 - pan.1) * k) };
    }

    /// One step in or out.
    pub fn step(&mut self, path: &str, inward: bool, at: (f32, f32)) {
        let (now, _) = self.of(path);
        let next = if inward { ZOOMS.iter().copied().find(|z| *z > now + 0.01) } else { ZOOMS.iter().rev().copied().find(|z| *z < now - 0.01) };
        if let Some(next) = next {
            self.set(path, next, at);
        }
    }

    /// A pinch's share, as the trackpad sends one each frame: the scale grows
    /// by `delta` of itself (0.1: a tenth bigger), the point `at` kept under
    /// the fingers. Let go a hair past whole, it settles whole, so the bar
    /// says Fit rather than 101%.
    pub fn pinch(&mut self, path: &str, delta: f32, at: (f32, f32), ended: bool) {
        let (now, _) = self.of(path);
        let next = if delta.is_finite() { now * (1. + delta).max(0.05) } else { now };
        self.set(path, next, at);
        if ended && self.scale < PINCH_SETTLE {
            self.set(path, 1., at);
        }
    }

    /// Kept so the picture (`fit` at scale 1) still covers the view (`room`)
    /// where it is bigger, and stays centred where it isn't.
    pub fn clamp(&mut self, fit: (f32, f32), room: (f32, f32)) {
        let reach = |side: f32, room: f32| ((side * self.scale - room) / 2.).max(0.);
        let (x, y) = (reach(fit.0, room.0), reach(fit.1, room.1));
        self.pan = (self.pan.0.clamp(-x, x), self.pan.1.clamp(-y, y));
    }
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

    /// A reply's pictures — `(path, what it is called)`, each once — open at `path`.
    pub fn of_reply(row: u64, pictures: &[(String, String)], path: &str) -> Option<Self> {
        let mut items: Vec<Item> = Vec::new();
        for (p, label) in pictures {
            if !items.iter().any(|i| i.path == *p) {
                let label = if label.trim().is_empty() { name_of(p).to_owned() } else { label.clone() };
                items.push(Item { path: p.clone(), choice: None, label });
            }
        }
        let at = items.iter().position(|i| i.path == path)?;
        Some(Self { owner: Owner::Reply(row), items, at })
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

    #[test]
    fn a_short_card_shows_a_whole_row_of_smaller_tiles_rather_than_cut_captions() {
        let roomy = grid_within(900., 2, true, 2000.);
        assert_eq!(roomy, grid(900., 2), "room enough: as before");
        let short = grid_within(900., 2, true, 260.);
        assert!(tile_height(short.tile, true) <= 260., "{short:?}");
        assert!(short.tile < roomy.tile && short.rows == vec![2], "{short:?}");
        // Never smaller than a tile can be read at.
        assert_eq!(grid_within(900., 3, true, 40.).tile, TILE_SHRUNK);
    }

    #[test]
    fn the_big_view_zooms_in_steps_toward_the_pointer_and_never_off_the_picture() {
        let mut z = Zoom::default();
        assert_eq!(z.of("a.png"), (1., (0., 0.)), "whole to start");
        // In a step at the centre: no pan.
        z.step("a.png", true, (0., 0.));
        assert_eq!(z.of("a.png"), (1.5, (0., 0.)));
        // At a point right of centre: that point stays under the pointer.
        z.set("a.png", 3., (100., 0.));
        let (scale, pan) = z.of("a.png");
        assert_eq!(scale, 3.);
        assert!((pan.0 - -100.).abs() < 0.01, "{pan:?}: the picture moves left by what grew past the pointer");
        // Kept to the picture: an 800-wide fit at 3× in a 1000 room reaches 700 each way.
        z.pan = (-5000., 40.);
        z.clamp((800., 400.), (1000., 600.));
        assert_eq!(z.pan, (-700., 40.));
        // A side that still fits stays centred.
        z.set("a.png", 1.5, (0., 0.));
        z.pan = (0., 90.);
        z.clamp((800., 400.), (1000., 600.));
        assert_eq!(z.pan.1, 0.);
        // Out to whole: centred again; and never past the ends.
        z.set("a.png", 0.2, (300., 300.));
        assert_eq!(z.of("a.png"), (1., (0., 0.)));
        z.step("a.png", false, (0., 0.));
        assert_eq!(z.of("a.png").0, 1.);
        z.set("a.png", 99., (0., 0.));
        assert_eq!(z.of("a.png").0, 12.);
        // Another picture starts whole.
        assert_eq!(z.of("b.png"), (1., (0., 0.)));
    }

    /// Where the point `at` (px from the view's centre) is on the picture
    /// itself, at scale 1: what must stay under the fingers.
    fn on_picture(z: &Zoom, path: &str, at: (f32, f32)) -> (f32, f32) {
        let (scale, pan) = z.of(path);
        ((at.0 - pan.0) / scale, (at.1 - pan.1) / scale)
    }

    #[test]
    fn a_pinch_zooms_frame_by_frame_keeping_what_is_under_the_fingers() {
        let mut z = Zoom::default();
        let (fit, room) = ((800., 500.), (1000., 600.));
        let at = (150., -60.);
        let under = on_picture(&z, "a.png", at);
        // Twenty small frames in: smooth, every one a little further, none a step.
        let mut last = 1.;
        for _ in 0..20 {
            z.pinch("a.png", 0.04, at, false);
            let (scale, _) = z.of("a.png");
            assert!(scale > last && scale - last < 0.1 * scale, "{last} → {scale}");
            last = scale;
            let now = on_picture(&z, "a.png", at);
            assert!((now.0 - under.0).abs() < 0.01 && (now.1 - under.1).abs() < 0.01, "{under:?} drifted to {now:?} at {scale}");
        }
        assert!((last - 1.04f32.powi(20)).abs() < 0.01, "{last}");
        // Kept over the view, it still holds where the picture is bigger than the view.
        z.clamp(fit, room);
        let under = on_picture(&z, "a.png", at);
        z.pinch("a.png", 0.04, at, false);
        z.clamp(fit, room);
        let now = on_picture(&z, "a.png", at);
        assert!((now.0 - under.0).abs() < 0.01 && (now.1 - under.1).abs() < 0.01, "{under:?} → {now:?}");
        // And out again at another point: that one stays put instead.
        let at = (-200., 100.);
        let under = on_picture(&z, "a.png", at);
        z.pinch("a.png", -0.1, at, false);
        z.clamp(fit, room);
        let now = on_picture(&z, "a.png", at);
        assert!((now.0 - under.0).abs() < 0.01 && (now.1 - under.1).abs() < 0.01, "{under:?} → {now:?}");
        // The + and − steps go on from wherever the pinch left it.
        let (pinched, _) = z.of("a.png");
        z.step("a.png", true, (0., 0.));
        assert_eq!(z.of("a.png").0, ZOOMS.iter().copied().find(|s| *s > pinched).unwrap());
    }

    #[test]
    fn a_pinch_stops_at_whole_and_at_the_most_and_settles_whole_when_let_go_near_it() {
        let mut z = Zoom::default();
        // Out past whole: whole, centred — the bar says Fit.
        z.pinch("a.png", -0.5, (300., 200.), false);
        assert_eq!(z.of("a.png"), (1., (0., 0.)));
        // In past the most: the most.
        for _ in 0..200 {
            z.pinch("a.png", 0.2, (0., 0.), false);
        }
        assert_eq!(z.of("a.png").0, ZOOMS[ZOOMS.len() - 1]);
        // A wild frame (a whole pinch's worth the other way) neither flips nor breaks it.
        z.pinch("a.png", -5., (0., 0.), false);
        assert_eq!(z.of("a.png"), (1., (0., 0.)));
        z.pinch("a.png", f32::NAN, (0., 0.), false);
        assert_eq!(z.of("a.png").0, 1.);
        // Let go a hair past whole: whole; let go well in: kept as it is.
        z.pinch("a.png", 0.02, (100., 0.), false);
        z.pinch("a.png", 0., (100., 0.), true);
        assert_eq!(z.of("a.png"), (1., (0., 0.)));
        z.pinch("a.png", 0.3, (100., 0.), false);
        z.pinch("a.png", 0., (100., 0.), true);
        assert!((z.of("a.png").0 - 1.3).abs() < 0.001);
        // Pinched out near the edge, the picture is kept over the view, not off it.
        let (fit, room) = ((800., 500.), (1000., 600.));
        z.set("a.png", 4., (0., 0.));
        z.pinch("a.png", -0.5, (-450., 0.), false);
        z.clamp(fit, room);
        let (scale, pan) = z.of("a.png");
        assert!(pan.0.abs() <= (fit.0 * scale - room.0) / 2. + 0.01, "{pan:?} at {scale}");
    }
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
        v.put("/w/gone.png", Size::Thumb, Look::Gone(ImageWhy::Missing));
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
    fn a_grid_balances_its_rows_and_never_leaves_one_tile_alone_when_it_could_be_helped() {
        let rows = |w: f32, n: usize| grid(w, n).rows;
        assert_eq!(rows(640., 4), [2, 2], "not three and one");
        assert_eq!(rows(640., 5), [3, 2]);
        assert_eq!(rows(640., 3), [3]);
        assert_eq!(rows(640., 7), [3, 2, 2]);
        assert_eq!(rows(640., 8), [3, 3, 2]);
        assert_eq!(rows(900., 8), [4, 4]);
        assert_eq!(rows(900., 5), [3, 2]);
        assert_eq!(rows(640., 1), [1]);
        assert_eq!(rows(400., 4), [2, 2], "narrow: two a row");
        assert_eq!(rows(300., 3), [1, 1, 1], "narrower: one");
        assert!(rows(640., 0).is_empty());
        // Wrapping keeps the tiles the size the rows that fit give them.
        assert_eq!(grid(640., 4).tile, grid(640., 3).tile);
        assert_eq!(grid(640., 2).tile, TILE_MAX, "all in one row: the row shared out");
        for n in 1..=12 {
            for w in [180., 300., 400., 520., 640., 760., 900., 1400.] {
                let g = grid(w, n);
                let (most, least) = (*g.rows.iter().max().unwrap(), *g.rows.iter().min().unwrap());
                assert_eq!(g.rows.iter().sum::<usize>(), n);
                assert!(most - least <= 1, "{w} {n}: {:?}", g.rows);
                assert!(most as f32 * g.tile + (most as f32 - 1.) * GAP <= w, "{w} {n}: no wider than the card");
                if columns(w, n) >= 3 && n > 1 {
                    assert!(least >= 2, "{w} {n}: {:?}", g.rows);
                }
            }
        }
    }

    #[test]
    fn picking_a_tile_or_pointing_at_it_moves_nothing() {
        let g = grid(640., 4);
        let h = 200.;
        let states = [
            TileState::default(),
            TileState { picked: true, ..TileState::default() },
            TileState { cursor: true, ..TileState::default() },
            TileState { hover: 1., ..TileState::default() },
            TileState { picked: true, cursor: true, hover: 0.5 },
        ];
        for ix in 0..4 {
            let at = g.rect(ix, h, TileState::default()).unwrap();
            for state in states {
                assert_eq!(g.rect(ix, h, state), Some(at), "tile {ix} in {state:?}");
                assert_eq!(frame(state).border, 1., "the border is one point in every state");
            }
        }
        assert_eq!(g.rect(2, h, states[1]), Some((0., h + GAP, g.tile, h)), "the third under the first");
        assert_eq!(g.rect(4, h, states[0]), None);
        assert!(frame(states[1]).ring > 0. && frame(states[0]).ring == 0., "picked: a ring, inside");
        assert_eq!(g.height(h), 2. * h + GAP);
        assert_eq!(g.spans(), [0..2, 2..4]);
        // Every tile of a question is one height, described or not.
        assert!(caption_height(true) > caption_height(false));
        assert_eq!(lead_box(900.), (LEAD_WIDE, LEAD_HIGH));
        assert_eq!(lead_box(320.), (320., 240.));
    }

    #[test]
    fn a_picture_in_a_reply_keeps_its_shape_within_the_column_and_never_grows() {
        assert_eq!(inline_box(Some((2400, 1600)), 600.), (600., 400.), "the column's width, its own shape");
        assert_eq!(inline_box(Some((1000, 2000)), 600.), (210., 420.), "a tall one stops at the height");
        assert_eq!(inline_box(Some((300, 200)), 600.), (300., 200.), "a small one as it is");
        assert_eq!(inline_box(Some((2400, 1600)), 320.), (320., 213.), "a narrow window");
        assert_eq!(inline_box(None, 600.), (480., 360.), "unknown: 4:3, a guess");
        assert_eq!(inline_box(None, 300.), (300., 225.));
    }

    #[test]
    fn a_folder_is_listed_once_and_its_rows_are_remembered() {
        let mut v = Visuals::default();
        assert_eq!(v.listing("/w/images"), Listing::Loading);
        assert_eq!(v.listing("/w/images"), Listing::Loading);
        assert!(v.has_wanted());
        assert_eq!(v.take_wanted_folders(), ["/w/images"]);
        v.put_listing("/w/images", Listing::Ready(ImageList { folder: "/w/images".into(), images: vec!["/w/images/a.png".into()], total: 1 }));
        assert!(matches!(v.listing("/w/images"), Listing::Ready(l) if l.total == 1));
        v.put_listing("/w/gone", Listing::Gone);
        v.retry_gone();
        assert_eq!(v.listing("/w/gone"), Listing::Loading, "asked again once back on the line");
        v.in_row("/w/a.png", 7);
        v.in_row("/w/a.png", 9);
        let mut rows = v.rows_of("/w/a.png");
        rows.sort();
        assert_eq!(rows, [7, 9]);
    }

    #[test]
    fn a_gallery_whose_pictures_all_failed_reads_as_one_line_with_the_reason_most_share() {
        let missing = Look::Gone(ImageWhy::Missing);
        let big = Look::Gone(ImageWhy::TooBig);
        let ready = shot(1);
        assert_eq!(all_failed(&[Some(&missing), Some(&missing), Some(&big)]), Some(ImageWhy::Missing));
        assert_eq!(all_failed(&[Some(&missing), Some(&ready)]), None, "one came: each says its own");
        assert_eq!(all_failed(&[Some(&missing), None]), None, "one still coming");
        assert_eq!(all_failed(&[]), None);
        let mut v = Visuals::default();
        v.put("/w/a.png", Size::Thumb, Look::Gone(ImageWhy::Missing));
        v.put("/w/b.png", Size::Thumb, Look::Gone(ImageWhy::Missing));
        v.retry(&["/w/a.png".to_owned()]);
        assert!(v.known("/w/a.png", Size::Thumb).is_none() && v.is_gone("/w/b.png"), "only what was asked again");
    }

    #[test]
    fn a_hand_ins_assets_come_after_its_work_each_once() {
        let images: Vec<String> = ["/w/public/textures/earth_day_4096.jpg", "/w/outputs/T-4-globe.png", "/w/public/textures/earth_day_2048.jpg", "/w/outputs/T-4-flying.png"].map(str::to_owned).to_vec();
        let (work, assets) = arrange(&images);
        assert_eq!(work, ["/w/outputs/T-4-globe.png", "/w/outputs/T-4-flying.png"]);
        assert_eq!(assets, ["/w/public/textures/earth_day_4096.jpg"]);
    }

    #[test]
    fn the_big_view_goes_through_a_replys_pictures_once_each() {
        let pictures = [("/w/plan.png".to_owned(), "평면도".to_owned()), ("/w/site.png".to_owned(), String::new()), ("/w/plan.png".to_owned(), "again".to_owned())];
        let mut v = Viewer::of_reply(3, &pictures, "/w/site.png").unwrap();
        assert_eq!((v.items.len(), v.at, v.item().label.as_str()), (2, 1, "site.png"));
        v.step(true);
        assert_eq!((v.item().label.as_str(), v.owner.clone()), ("평면도", Owner::Reply(3)));
        assert!(Viewer::of_reply(3, &pictures, "/w/other.png").is_none());
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
