//! The taste board's rules (`views/taste.rs` draws it): a question whose
//! options are families of references, browsed as one masonry of pictures —
//! how many columns a width has, where every tile goes (variable heights, the
//! shortest column first, each family under its own header), which tile each
//! arrow moves to, and what every key does. Likes and the family picked live
//! in the question's draft (`crate::decide::Draft`), so the card and the board
//! never disagree. No UI types here, so the rules are tested without a window.

/// The space between tiles, and under a family's header.
pub const GAP: f32 = 12.;
/// A tile is never narrower than this: past it, one column fewer.
pub const MIN_TILE: f32 = 200.;
/// At least two columns, at most five.
pub const COLUMNS: std::ops::RangeInclusive<usize> = 2..=5;
/// A tile's height for its width when its picture's size isn't known (4:3).
pub const DEFAULT_RATIO: f32 = 0.75;
/// The flattest and the tallest a tile gets, as height over width: a
/// panorama or a long page is still one tile, not a strip or a tower.
pub const RATIO_MIN: f32 = 0.4;
pub const RATIO_MAX: f32 = 2.4;
/// A family's header: its name, its one line and its button.
pub const HEADER: f32 = 68.;
/// The space between one family and the next.
pub const GROUPS_GAP: f32 = 28.;

/// How many columns a board `width` points wide has: as many as fit at
/// [`MIN_TILE`] each, two to five.
pub fn columns_for(width: f32) -> usize {
    let fit = ((width + GAP) / (MIN_TILE + GAP)).floor();
    (if fit.is_finite() && fit > 0. { fit as usize } else { 1 }).clamp(*COLUMNS.start(), *COLUMNS.end())
}

/// A box in points.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn cx(&self) -> f32 {
        self.x + self.w / 2.
    }

    pub fn cy(&self) -> f32 {
        self.y + self.h / 2.
    }
}

/// How tall a tile `width` wide is for a picture `ratio` (height over width).
pub fn tile_height(width: f32, ratio: f32) -> f32 {
    let ratio = if ratio.is_finite() && ratio > 0. { ratio.clamp(RATIO_MIN, RATIO_MAX) } else { DEFAULT_RATIO };
    (width * ratio).round().max(1.)
}

/// A picture's height over its width, from its size in pixels; none given
/// (or a size without sides), the default.
pub fn ratio_of(dims: Option<(u32, u32)>) -> f32 {
    match dims {
        Some((w, h)) if w > 0 && h > 0 => h as f32 / w as f32,
        _ => DEFAULT_RATIO,
    }
}

/// Tiles of one family laid out: each in the shortest column, left to right
/// on a tie, all one width.
#[derive(Debug, Clone, PartialEq)]
pub struct Masonry {
    pub tile: f32,
    pub rects: Vec<Rect>,
    pub height: f32,
}

/// `ratios` (a tile's height over its width, in order) in `columns` columns
/// within `width`.
pub fn masonry(width: f32, columns: usize, ratios: &[f32]) -> Masonry {
    let columns = columns.max(1);
    let tile = ((width - GAP * (columns as f32 - 1.)) / columns as f32).floor().max(1.);
    let mut tops = vec![0f32; columns];
    let mut rects = Vec::with_capacity(ratios.len());
    for ratio in ratios {
        let column = tops.iter().enumerate().fold(0, |best, (i, top)| if *top < tops[best] { i } else { best });
        let h = tile_height(tile, *ratio);
        rects.push(Rect { x: column as f32 * (tile + GAP), y: tops[column], w: tile, h });
        tops[column] += h + GAP;
    }
    let height = tops.iter().copied().fold(0., f32::max);
    let height = if rects.is_empty() { 0. } else { height - GAP };
    Masonry { tile, rects, height }
}

/// One tile of the board: the `tile`th of option `option`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileRef {
    pub option: usize,
    pub tile: usize,
}

/// A tile and where it is, in the board's own space (its top left is 0, 0).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub at: TileRef,
    pub rect: Rect,
}

/// A family on the board: its header, then its tiles.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub option: usize,
    /// Where its header starts, and its tiles' top.
    pub y: f32,
    pub body: f32,
    /// Its height, header and tiles.
    pub height: f32,
}

/// The whole board.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layout {
    pub columns: usize,
    pub tile: f32,
    pub groups: Vec<Group>,
    pub tiles: Vec<Placed>,
    pub height: f32,
}

impl Layout {
    pub fn rect_of(&self, at: TileRef) -> Option<Rect> {
        self.tiles.iter().find(|p| p.at == at).map(|p| p.rect)
    }

    /// Every tile on show, in reading order: family by family.
    pub fn order(&self) -> Vec<TileRef> {
        self.tiles.iter().map(|p| p.at).collect()
    }
}

/// The board `width` wide: each family's tiles (`families`, one list of
/// ratios each) in a masonry under its header, one family under the next —
/// only option `only`'s when a filter is set.
pub fn layout(width: f32, only: Option<usize>, families: &[Vec<f32>]) -> Layout {
    let columns = columns_for(width);
    let mut out = Layout { columns, ..Layout::default() };
    let mut y = 0f32;
    for (option, ratios) in families.iter().enumerate().filter(|(o, _)| only.is_none_or(|f| f == *o)) {
        let m = masonry(width, columns, ratios);
        let body = y + HEADER + GAP;
        out.tile = m.tile;
        out.groups.push(Group { option, y, body, height: HEADER + if m.rects.is_empty() { 0. } else { GAP + m.height } });
        for (tile, rect) in m.rects.iter().enumerate() {
            out.tiles.push(Placed { at: TileRef { option, tile }, rect: Rect { y: rect.y + body, ..*rect } });
        }
        y = if m.rects.is_empty() { y + HEADER + GROUPS_GAP } else { body + m.height + GROUPS_GAP };
    }
    out.height = (y - GROUPS_GAP).max(0.);
    out
}

// ── moving ──

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

/// How much of `a`'s extent `b` overlaps, in points.
fn overlap(a: (f32, f32), b: (f32, f32)) -> f32 {
    (a.1.min(b.1) - a.0.max(b.0)).max(0.)
}

/// The tile an arrow moves to from `from` (the first, when there is none
/// yet): Up and Down stay in the column — into the family above or below, in
/// the column where they have a tile, else the nearest —, Left and Right go
/// to the next column over, at about the same height. At an edge, nowhere:
/// the tile it is on.
pub fn step(tiles: &[Placed], from: Option<TileRef>, dir: Dir) -> Option<TileRef> {
    let first = tiles.first().map(|p| p.at);
    let Some(from) = from else { return first };
    let Some(here) = tiles.iter().find(|p| p.at == from) else { return first };
    let r = here.rect;
    let others = || tiles.iter().filter(|p| p.at != from);
    let best = match dir {
        Dir::Down | Dir::Up => {
            let down = dir == Dir::Down;
            let beyond = |p: &&Placed| if down { p.rect.y >= r.bottom() - 0.5 } else { p.rect.bottom() <= r.y + 0.5 };
            // Nearest in the column first (a tile over most of its width), else the nearest at all.
            let near = |p: &Placed| if down { p.rect.y } else { -p.rect.bottom() };
            let by = |a: &&Placed, b: &&Placed| near(a).total_cmp(&near(b)).then((a.rect.cx() - r.cx()).abs().total_cmp(&(b.rect.cx() - r.cx()).abs()));
            let aligned = others().filter(beyond).filter(|p| overlap((p.rect.x, p.rect.right()), (r.x, r.right())) >= r.w * 0.5).min_by(by);
            aligned.or_else(|| others().filter(beyond).min_by(by))
        }
        Dir::Left | Dir::Right => {
            let right = dir == Dir::Right;
            let beyond = |p: &&Placed| if right { p.rect.x >= r.right() - 0.5 } else { p.rect.right() <= r.x + 0.5 };
            // The next column over, then the tile at the nearest height — within sight of this one's height first.
            let across = |p: &Placed| if right { p.rect.x } else { -p.rect.x };
            let by = |a: &&Placed, b: &&Placed| across(a).total_cmp(&across(b)).then((a.rect.cy() - r.cy()).abs().total_cmp(&(b.rect.cy() - r.cy()).abs()));
            let seen = others().filter(beyond).filter(|p| overlap((p.rect.y, p.rect.bottom()), (r.y, r.bottom())) > 0.).min_by(by);
            seen.or_else(|| {
                let nearest = others().filter(beyond).map(across).min_by(f32::total_cmp)?;
                others().filter(beyond).filter(|p| (across(p) - nearest).abs() < 0.5).min_by(|a, b| (a.rect.cy() - r.cy()).abs().total_cmp(&(b.rect.cy() - r.cy()).abs()))
            })
        }
    };
    Some(best.map_or(from, |p| p.at))
}

// ── keys ──

/// A key the board takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Move(Dir),
    /// Space: like the tile the keys are on, or take the like back.
    Like,
    /// ⏎: open it big.
    Open,
    /// P: pick the family of that tile.
    Pick,
    /// ⌘⏎: send the answer.
    Send,
    /// Esc: back to the question.
    Back,
    /// 0: all families; 1–9: that one.
    Filter(usize),
    /// Tab: into the box for a note.
    Note,
}

/// The key a keystroke is, if the board takes it.
pub fn key_of(key: &str, cmd: bool, shift: bool) -> Option<Key> {
    if cmd {
        return (key == "enter").then_some(Key::Send);
    }
    if shift {
        return None;
    }
    Some(match key {
        "left" => Key::Move(Dir::Left),
        "right" => Key::Move(Dir::Right),
        "up" => Key::Move(Dir::Up),
        "down" => Key::Move(Dir::Down),
        "space" => Key::Like,
        "enter" => Key::Open,
        "p" => Key::Pick,
        "escape" => Key::Back,
        "tab" => Key::Note,
        d if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() => Key::Filter((d.as_bytes()[0] - b'0') as usize),
        _ => return None,
    })
}

/// What a key did, for the window to follow up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Not the board's key (or nowhere to go).
    Nothing,
    /// The keys moved: show the tile.
    Moved,
    Like(TileRef),
    Open(TileRef),
    /// Pick this family.
    Pick(usize),
    Send,
    Back,
    /// The filter changed: the family on show only, or all.
    Filter(Option<usize>),
    Note,
}

/// Where the person is on a board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Board {
    /// The set and question it shows.
    pub set: u64,
    pub step: usize,
    /// One family only, or all.
    pub filter: Option<usize>,
    /// The tile the keys are on.
    pub cursor: Option<TileRef>,
}

impl Board {
    pub fn new(set: u64, step: usize) -> Self {
        Self { set, step, filter: None, cursor: None }
    }

    /// The family the keys are on, if any: the tile's, else the one filtered to.
    pub fn family(&self) -> Option<usize> {
        self.cursor.map(|c| c.option).or(self.filter)
    }

    /// The filter set to `only` (a family, or all), the keys on its first tile.
    pub fn filter_to(&mut self, only: Option<usize>, tiles: &[Placed]) {
        self.filter = only;
        self.cursor = tiles.iter().map(|p| p.at).find(|at| only.is_none_or(|o| at.option == o));
    }

    /// A key, over the board's visible `tiles` of `families` families.
    pub fn key(&mut self, key: Key, tiles: &[Placed], families: usize) -> Effect {
        match key {
            Key::Move(dir) => {
                let to = step(tiles, self.cursor, dir);
                if to == self.cursor || to.is_none() {
                    return Effect::Nothing;
                }
                self.cursor = to;
                Effect::Moved
            }
            Key::Like => self.cursor.map_or(Effect::Nothing, Effect::Like),
            Key::Open => self.cursor.map_or(Effect::Nothing, Effect::Open),
            Key::Pick => self.family().map_or(Effect::Nothing, Effect::Pick),
            Key::Send => Effect::Send,
            Key::Back => Effect::Back,
            Key::Note => Effect::Note,
            Key::Filter(n) if n > families => Effect::Nothing,
            Key::Filter(n) => {
                let only = n.checked_sub(1);
                if only == self.filter {
                    return Effect::Nothing;
                }
                // The tiles the new filter shows aren't known here: the window
                // lays them out and settles the cursor (`filter_to`).
                self.filter = only;
                self.cursor = None;
                Effect::Filter(only)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(option: usize, tile: usize) -> TileRef {
        TileRef { option, tile }
    }

    #[test]
    fn columns_follow_the_width_two_to_five() {
        assert_eq!(columns_for(300.), 2, "never fewer than two");
        assert_eq!(columns_for(0.), 2);
        assert_eq!(columns_for(f32::NAN), 2);
        assert_eq!(columns_for(2. * MIN_TILE + GAP), 2);
        assert_eq!(columns_for(3. * MIN_TILE + 2. * GAP - 1.), 2, "a tile a point too narrow: a column fewer");
        assert_eq!(columns_for(3. * MIN_TILE + 2. * GAP), 3);
        assert_eq!(columns_for(1000.), 4);
        assert_eq!(columns_for(1400.), 5);
        assert_eq!(columns_for(4000.), 5, "never more than five");
    }

    #[test]
    fn a_tile_keeps_its_pictures_shape_within_limits() {
        assert_eq!(tile_height(200., 0.5), 100.);
        assert_eq!(tile_height(200., ratio_of(Some((1600, 1200)))), 150.);
        assert_eq!(tile_height(200., ratio_of(None)), 150., "size unknown: 4:3");
        assert_eq!(tile_height(200., ratio_of(Some((0, 10)))), 150.);
        assert_eq!(tile_height(200., ratio_of(Some((2400, 240)))), 80., "a panorama is no thinner than the floor");
        assert_eq!(tile_height(200., ratio_of(Some((240, 2400)))), 480., "a tower is no taller than the ceiling");
        assert_eq!(tile_height(200., f32::NAN), 150.);
    }

    #[test]
    fn the_masonry_puts_each_tile_in_the_shortest_column_and_shares_the_width() {
        // Width 3 * 100 + 2 * 12 = 324: three columns of 100.
        let m = masonry(324., 3, &[1., 0.5, 0.75, 0.5, 1.]);
        assert_eq!(m.tile, 100.);
        let at = |i: usize| (m.rects[i].x, m.rects[i].y, m.rects[i].h);
        assert_eq!(at(0), (0., 0., 100.));
        assert_eq!(at(1), (112., 0., 50.));
        assert_eq!(at(2), (224., 0., 75.));
        // The shortest is column 2 (62 tall: 50 + 12), then column 3 (87), then column 1 (112).
        assert_eq!(at(3), (112., 62., 50.));
        assert_eq!(at(4), (224., 87., 100.));
        assert_eq!(m.height, 187., "the tallest column, less the gap after it");
        // Nothing is laid on another.
        for (i, a) in m.rects.iter().enumerate() {
            for b in &m.rects[i + 1..] {
                let apart = a.right() <= b.x || b.right() <= a.x || a.bottom() <= b.y || b.bottom() <= a.y;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
        assert_eq!(masonry(324., 3, &[]), Masonry { tile: 100., rects: vec![], height: 0. });
        assert_eq!(masonry(10., 3, &[1.]).tile, 1., "never a tile of nothing");
    }

    #[test]
    fn a_board_stacks_its_families_each_under_a_header_and_a_filter_shows_one() {
        let families = vec![vec![1., 0.5], vec![0.75], vec![]];
        let all = layout(324., None, &families);
        assert_eq!((all.columns, all.tile), (2, 156.), "324 is two columns of 156");
        assert_eq!(all.groups.iter().map(|g| g.option).collect::<Vec<_>>(), [0, 1, 2]);
        let first = &all.groups[0];
        assert_eq!((first.y, first.body), (0., HEADER + GAP));
        // First family: tiles of 156 and 78, side by side; its masonry is 156 high.
        assert_eq!(all.tiles[0].rect, Rect { x: 0., y: HEADER + GAP, w: 156., h: 156. });
        assert_eq!(all.tiles[1].rect, Rect { x: 168., y: HEADER + GAP, w: 156., h: 78. });
        assert_eq!(first.height, HEADER + GAP + 156.);
        // The second starts a gap under the first.
        let second = &all.groups[1];
        assert_eq!(second.y, first.body + 156. + GROUPS_GAP);
        assert_eq!(all.tiles[2].at, at(1, 0));
        assert_eq!(all.tiles[2].rect.y, second.body);
        // A family with no tile is its header only; the board ends where the last does.
        assert_eq!(all.groups[2].height, HEADER);
        assert_eq!(all.height, all.groups[2].y + HEADER);
        // Filtered: only that family, from the top.
        let one = layout(324., Some(1), &families);
        assert_eq!(one.groups.len(), 1);
        assert_eq!(one.tiles.len(), 1);
        assert_eq!(one.tiles[0].rect.y, HEADER + GAP);
        assert_eq!(one.height, HEADER + GAP + 117.);
        assert_eq!(layout(324., None, &[]).height, 0.);
    }

    /// Three columns of 100, five tiles: the first the tallest.
    fn five() -> Vec<Placed> {
        let m = masonry(324., 3, &[1., 0.5, 0.75, 0.5, 1.]);
        m.rects.iter().enumerate().map(|(i, r)| Placed { at: at(0, i), rect: *r }).collect()
    }

    #[test]
    fn arrows_move_by_columns_and_stop_at_the_edges() {
        let tiles = five();
        // From the first: down the column — nothing under it; right to the second.
        assert_eq!(step(&tiles, Some(at(0, 0)), Dir::Down), Some(at(0, 0)));
        assert_eq!(step(&tiles, Some(at(0, 0)), Dir::Right), Some(at(0, 1)));
        // Second column: down to the one under it; up again.
        assert_eq!(step(&tiles, Some(at(0, 1)), Dir::Down), Some(at(0, 3)));
        assert_eq!(step(&tiles, Some(at(0, 3)), Dir::Up), Some(at(0, 1)));
        // Third column: the tall one's neighbours.
        assert_eq!(step(&tiles, Some(at(0, 2)), Dir::Down), Some(at(0, 4)));
        assert_eq!(step(&tiles, Some(at(0, 2)), Dir::Right), Some(at(0, 2)), "the last column: stay");
        assert_eq!(step(&tiles, Some(at(0, 0)), Dir::Left), Some(at(0, 0)));
        assert_eq!(step(&tiles, Some(at(0, 0)), Dir::Up), Some(at(0, 0)));
        // From the lower tile of the middle column, left lands on the first column's tile it faces.
        assert_eq!(step(&tiles, Some(at(0, 3)), Dir::Left), Some(at(0, 0)));
        assert_eq!(step(&tiles, Some(at(0, 3)), Dir::Right), Some(at(0, 2)), "the third column's tile nearest its height");
        // No cursor yet: the first tile, any way.
        for dir in [Dir::Left, Dir::Right, Dir::Up, Dir::Down] {
            assert_eq!(step(&tiles, None, dir), Some(at(0, 0)));
        }
        assert_eq!(step(&[], None, Dir::Down), None);
        assert_eq!(step(&tiles, Some(at(7, 7)), Dir::Down), Some(at(0, 0)), "a tile no longer there: start over");
    }

    #[test]
    fn arrows_cross_from_one_family_into_the_next() {
        let families = vec![vec![1., 1., 1.], vec![1., 1.]];
        let all = layout(324., None, &families);
        // Two columns: family 0 has (0,0) (0,1) over (0,2); family 1 starts below.
        assert_eq!(step(&all.tiles, Some(at(0, 2)), Dir::Down), Some(at(1, 0)), "down from the last of a column into the next family's column");
        assert_eq!(step(&all.tiles, Some(at(0, 1)), Dir::Down), Some(at(1, 1)), "a column with nothing under it: the next family's");
        assert_eq!(step(&all.tiles, Some(at(1, 0)), Dir::Up), Some(at(0, 2)));
        // Right never leaves the family it is in while its row has a tile there.
        assert_eq!(step(&all.tiles, Some(at(1, 0)), Dir::Right), Some(at(1, 1)));
    }

    #[test]
    fn keys_do_what_the_board_says() {
        assert_eq!(key_of("left", false, false), Some(Key::Move(Dir::Left)));
        assert_eq!(key_of("space", false, false), Some(Key::Like));
        assert_eq!(key_of("enter", false, false), Some(Key::Open));
        assert_eq!(key_of("enter", true, false), Some(Key::Send), "⌘⏎ sends");
        assert_eq!(key_of("left", true, false), None, "no other ⌘ key is the board's");
        assert_eq!(key_of("left", false, true), None);
        assert_eq!(key_of("escape", false, false), Some(Key::Back));
        assert_eq!(key_of("3", false, false), Some(Key::Filter(3)));
        assert_eq!(key_of("x", false, false), None);

        let tiles = five();
        let mut board = Board::new(9, 0);
        assert_eq!(board.key(Key::Like, &tiles, 1), Effect::Nothing, "nothing to like before the keys are on a tile");
        assert_eq!(board.key(Key::Move(Dir::Right), &tiles, 1), Effect::Moved);
        assert_eq!(board.cursor, Some(at(0, 0)));
        assert_eq!(board.key(Key::Move(Dir::Right), &tiles, 1), Effect::Moved);
        assert_eq!(board.key(Key::Move(Dir::Right), &tiles, 1), Effect::Moved);
        assert_eq!(board.key(Key::Move(Dir::Right), &tiles, 1), Effect::Nothing, "the edge");
        assert_eq!(board.cursor, Some(at(0, 2)));
        assert_eq!(board.key(Key::Like, &tiles, 1), Effect::Like(at(0, 2)));
        assert_eq!(board.key(Key::Open, &tiles, 1), Effect::Open(at(0, 2)));
        assert_eq!(board.key(Key::Pick, &tiles, 1), Effect::Pick(0));
        assert_eq!(board.key(Key::Send, &tiles, 1), Effect::Send);
        assert_eq!(board.key(Key::Back, &tiles, 1), Effect::Back);
        assert_eq!(board.key(Key::Note, &tiles, 1), Effect::Note);
    }

    #[test]
    fn digits_filter_to_a_family_and_zero_shows_all() {
        let mut board = Board::new(1, 0);
        assert_eq!(board.key(Key::Filter(2), &[], 3), Effect::Filter(Some(1)));
        assert_eq!((board.filter, board.cursor), (Some(1), None));
        assert_eq!(board.key(Key::Filter(2), &[], 3), Effect::Nothing, "already there");
        assert_eq!(board.key(Key::Filter(4), &[], 3), Effect::Nothing, "no fourth family");
        assert_eq!(board.key(Key::Filter(0), &[], 3), Effect::Filter(None));
        // The window settles the cursor on the first tile on show.
        let families = vec![vec![1.], vec![1., 1.]];
        let shown = layout(324., Some(1), &families);
        board.filter_to(Some(1), &shown.tiles);
        assert_eq!(board.cursor, Some(at(1, 0)));
        assert_eq!(board.family(), Some(1));
        assert_eq!(board.key(Key::Pick, &shown.tiles, 2), Effect::Pick(1));
        // With the keys on no tile, P takes the family filtered to.
        board.cursor = None;
        assert_eq!(board.key(Key::Pick, &shown.tiles, 2), Effect::Pick(1));
        board.filter = None;
        assert_eq!(board.key(Key::Pick, &shown.tiles, 2), Effect::Nothing);
    }
}
