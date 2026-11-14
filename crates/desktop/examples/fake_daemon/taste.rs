//! `FAKE_DECIDE=board`: the designer puts a taste board to the person — three
//! families of references, each a set of pictures of different shapes, every
//! one with its source and what to take from it. The pictures are generated
//! here, wireframes of invented screens in each family's manner (a dense
//! tool, calm finance, a data dashboard), drawn into the same folder as the
//! other decisions' pictures (`$TMPDIR/equipe-fake/pictures`) and read back as
//! the daemon reads a member's pictures. Nothing in them is another product's:
//! no screenshots, no logos, no words — only boxes and bars. The sources are
//! `.example` addresses, which belong to nobody.

use std::path::Path;

use equipe_core::protocol::decide::{Choice, Question, Tile};
use image::{Rgba, RgbaImage};

/// How wide every picture is, and how tall each family's are, in pixels.
const W: u32 = 640;
const HEIGHTS: [&[u32]; 3] = [&[480, 640, 400, 560, 720, 420, 600, 520, 680], &[560, 420, 700, 480, 640, 400, 760, 520], &[440, 600, 500, 720, 400, 560, 640, 480, 760, 520]];

/// Whether `(x, y)` is in the box from `(x0, y0)` to `(x1, y1)`, its corners `r` round.
fn rr(x: f32, y: f32, (x0, y0, x1, y1): (f32, f32, f32, f32), r: f32) -> bool {
    if x < x0 || x > x1 || y < y0 || y > y1 {
        return false;
    }
    let r = r.min((x1 - x0) / 2.).min((y1 - y0) / 2.);
    let cx = x.clamp(x0 + r, (x1 - r).max(x0 + r));
    let cy = y.clamp(y0 + r, (y1 - r).max(y0 + r));
    (x - cx).powi(2) + (y - cy).powi(2) <= r * r
}

/// A picture `w` by `h` painted pixel by pixel: `paint(x, y)` is the colour at
/// a point, or none for the background.
fn draw(w: u32, h: u32, background: [u8; 4], paint: impl Fn(f32, f32) -> Option<[u8; 4]>) -> RgbaImage {
    RgbaImage::from_fn(w, h, |px, py| {
        // Two samples across and down, so rounded edges are smooth.
        let mut sum = [0f32; 3];
        for (dx, dy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
            let c = paint(px as f32 + dx, py as f32 + dy).unwrap_or(background);
            for (s, v) in sum.iter_mut().zip(c) {
                *s += v as f32;
            }
        }
        Rgba([(sum[0] / 4.) as u8, (sum[1] / 4.) as u8, (sum[2] / 4.) as u8, 0xFF])
    })
}

/// A small pseudo-random number from two seeds: the same pictures every run.
fn noise(a: usize, b: usize) -> f32 {
    let n = (a as u32).wrapping_mul(2_654_435_761) ^ (b as u32).wrapping_mul(40_503).wrapping_add(0x9E37_79B9);
    ((n ^ (n >> 15)).wrapping_mul(2_246_822_519) >> 8) as f32 / (1u32 << 24) as f32
}

/// A dense tool: a dark sidebar of rows, a list of tight rows with a dot, a
/// line of words and a chip each, one accent.
fn dense(n: usize, h: u32) -> RgbaImage {
    let (wf, hf) = (W as f32, h as f32);
    let accent = [0x5E, 0x6A, 0xD2, 0xFF];
    let dots: [[u8; 4]; 4] = [[0xE5, 0x6B, 0x5D, 0xFF], [0xE9, 0xB2, 0x4A, 0xFF], accent, [0x4C, 0xB7, 0x82, 0xFF]];
    draw(W, h, [0x15, 0x17, 0x1C, 0xFF], move |x, y| {
        if x < 118. {
            for i in 0..((hf - 40.) / 26.) as usize {
                let top = 28. + i as f32 * 26.;
                let wide = 56. + noise(n, i) * 34.;
                if rr(x, y, (16., top, 16. + wide, top + 11.), 4.) {
                    return Some(if i == n % 6 { accent } else { [0x39, 0x3F, 0x4A, 0xFF] });
                }
            }
            return Some([0x1B, 0x1E, 0x24, 0xFF]);
        }
        // The title and its toolbar.
        if rr(x, y, (140., 24., 140. + 150., 38.), 5.) {
            return Some([0xE4, 0xE6, 0xEB, 0xFF]);
        }
        for k in 0..3 {
            if rr(x, y, (wf - 36. - k as f32 * 54., 22., wf - 36. - k as f32 * 54. + 44., 42.), 6.) {
                return Some([0x2A, 0x2E, 0x37, 0xFF]);
            }
        }
        // The rows: a thin rule under each.
        let top = 66.;
        if y >= top && y < hf - 12. {
            let row = ((y - top) / 24.) as usize;
            let local = (y - top) % 24.;
            if local >= 23. {
                return Some([0x24, 0x28, 0x30, 0xFF]);
            }
            let cy = 12.;
            if (x - 150.).powi(2) + (local - cy).powi(2) <= 16. {
                return Some(dots[(row + n) % 4]);
            }
            let text = 70. + noise(n * 7 + row, 3) * 190.;
            if rr(x, local, (168., 8., 168. + text, 16.), 3.) {
                return Some([0xB8, 0xBD, 0xC8, 0xFF]);
            }
            if rr(x, local, (168. + text + 12., 9., 168. + text + 12. + 34., 15.), 3.) {
                return Some([0x2F, 0x34, 0x3E, 0xFF]);
            }
            if rr(x, local, (wf - 92., 7., wf - 36., 17.), 4.) && (row + n) % 3 == 0 {
                return Some([0x3A, 0x41, 0x5E, 0xFF]);
            }
        }
        None
    })
}

/// Calm finance: a warm light page, one big number, soft cards, a thin line.
fn calm(n: usize, h: u32) -> RgbaImage {
    let (wf, hf) = (W as f32, h as f32);
    let green = [0x3E, 0x7C, 0x6B, 0xFF];
    draw(W, h, [0xF6, 0xF3, 0xEC, 0xFF], move |x, y| {
        let ink = [0x2B, 0x2A, 0x28, 0xFF];
        if rr(x, y, (48., 52., 48. + 190. + noise(n, 1) * 60., 100.), 14.) {
            return Some(ink);
        }
        if rr(x, y, (48., 116., 48. + 150., 128.), 6.) {
            return Some([0xA8, 0xA2, 0x95, 0xFF]);
        }
        // The card: a hairline, then white, a thin line across it.
        let card = (32., 160., wf - 32., (hf * 0.5).max(300.));
        if rr(x, y, card, 22.) {
            let mid = (card.1 + card.3) / 2.;
            let wave = mid + ((x / 70. + n as f32).sin() * 28. + (x / 23.).sin() * 6.) * (1. - (n % 3) as f32 * 0.2);
            if (y - wave).abs() < 1.8 && x > 56. && x < wf - 56. {
                return Some(green);
            }
            if !rr(x, y, (card.0 + 1., card.1 + 1., card.2 - 1., card.3 - 1.), 21.) {
                return Some([0xE6, 0xE1, 0xD6, 0xFF]);
            }
            return Some([0xFF, 0xFF, 0xFF, 0xFF]);
        }
        // Two small cards under it, a few quiet rows in each.
        let top = card.3 + 24.;
        for side in 0..2 {
            let x0 = 32. + side as f32 * ((wf - 64. - 16.) / 2. + 16.);
            let bx = (x0, top, x0 + (wf - 64. - 16.) / 2., hf - 28.);
            if bx.3 > bx.1 + 40. && rr(x, y, bx, 18.) {
                for row in 0..((bx.3 - bx.1 - 24.) / 30.) as usize {
                    let ry = bx.1 + 20. + row as f32 * 30.;
                    if rr(x, y, (x0 + 18., ry, x0 + 18. + 70. + noise(n + side, row) * 80., ry + 9.), 4.) {
                        return Some([0xC9, 0xC3, 0xB6, 0xFF]);
                    }
                }
                return Some(if !rr(x, y, (bx.0 + 1., bx.1 + 1., bx.2 - 1., bx.3 - 1.), 17.) { [0xE6, 0xE1, 0xD6, 0xFF] } else { [0xFF, 0xFF, 0xFF, 0xFF] });
            }
        }
        None
    })
}

/// A data dashboard: figures across the top, a chart of bars, a table.
fn data(n: usize, h: u32) -> RgbaImage {
    let (wf, hf) = (W as f32, h as f32);
    let purple = [0x6C, 0x4C, 0xF1, 0xFF];
    let edge = [0xE4, 0xE2, 0xF2, 0xFF];
    draw(W, h, [0xF4, 0xF3, 0xFB, 0xFF], move |x, y| {
        let panel = |b: (f32, f32, f32, f32)| rr(x, y, b, 12.);
        let inner = |b: (f32, f32, f32, f32)| rr(x, y, (b.0 + 1., b.1 + 1., b.2 - 1., b.3 - 1.), 11.);
        // The figures: four cards, a number, a bar of its change.
        for i in 0..4 {
            let b = (24. + i as f32 * 150., 24., 24. + i as f32 * 150. + 136., 104.);
            if panel(b) {
                if rr(x, y, (b.0 + 14., b.1 + 16., b.0 + 14. + 60., b.1 + 24.), 3.) {
                    return Some([0xB5, 0xB2, 0xCF, 0xFF]);
                }
                if rr(x, y, (b.0 + 14., b.1 + 38., b.0 + 14. + 70. + noise(n, i) * 24., b.1 + 58.), 5.) {
                    return Some([0x1F, 0x1B, 0x3A, 0xFF]);
                }
                if rr(x, y, (b.0 + 14., b.1 + 66., b.0 + 14. + 38., b.1 + 74.), 4.) {
                    return Some(if (i + n) % 3 == 0 { [0xE0, 0x5A, 0x6A, 0xFF] } else { [0x34, 0xB0, 0x7A, 0xFF] });
                }
                return Some(if inner(b) { [0xFF, 0xFF, 0xFF, 0xFF] } else { edge });
            }
        }
        // The chart: a dozen bars of two tones.
        let chart = (24., 124., wf - 24., 124. + (hf * 0.38).max(170.));
        if panel(chart) {
            for i in 0..12 {
                let full = chart.3 - chart.1 - 44.;
                let bar = 24. + noise(n * 13 + i, 5) * (full - 24.);
                let x0 = chart.0 + 28. + i as f32 * 46.;
                if rr(x, y, (x0, chart.3 - 20. - bar, x0 + 26., chart.3 - 20.), 5.) {
                    return Some(if i % 4 == 3 { [0xB8, 0xA9, 0xFA, 0xFF] } else { purple });
                }
            }
            return Some(if inner(chart) { [0xFF, 0xFF, 0xFF, 0xFF] } else { edge });
        }
        // The table: rows of a name, a bar and a status dot.
        let table = (24., chart.3 + 20., wf - 24., hf - 24.);
        if table.3 > table.1 + 50. && panel(table) {
            let row = ((y - table.1 - 14.) / 30.).floor();
            let local = (y - table.1 - 14.) - row * 30.;
            if row >= 0. && local < 30. {
                let r = row as usize;
                if rr(x, local, (table.0 + 18., 10., table.0 + 18. + 80. + noise(n + r, 9) * 70., 18.), 4.) {
                    return Some([0x4A, 0x46, 0x6E, 0xFF]);
                }
                if rr(x, local, (table.0 + 280., 11., table.0 + 280. + 30. + noise(r, n) * 150., 17.), 3.) {
                    return Some([0xD6, 0xD2, 0xEE, 0xFF]);
                }
                if (x - (wf - 56.)).powi(2) + (local - 14.).powi(2) <= 25. {
                    return Some(if r % 2 == 0 { [0x34, 0xB0, 0x7A, 0xFF] } else { [0xF0, 0xA8, 0x3C, 0xFF] });
                }
                if local >= 29. {
                    return Some(edge);
                }
            }
            return Some(if inner(table) { [0xFF, 0xFF, 0xFF, 0xFF] } else { edge });
        }
        None
    })
}

/// The tiles of each family: what to take from it, and where it is from.
const FAMILIES: [(&str, &str, &str, &[(&str, &str)]); 3] = [
    (
        "Dense tool",
        "Tight rows, a quiet sidebar, one accent and the keyboard first: a workbench for people who live in it",
        "dense.example",
        &[
            ("issues list", "28px rows, status dot first, one muted chip"),
            ("sidebar", "Collapsed groups, the active row the only colour"),
            ("board view", "Columns as plain lists, no cards inside cards"),
            ("command menu", "One box over everything, grouped results"),
            ("detail panel", "Properties on the right, one weight of text"),
            ("filters", "Filters as chips in the toolbar, not a sheet"),
            ("inbox", "A list that reads like mail, unread in bold"),
            ("cycle view", "Progress as a thin line under the title"),
            ("settings", "A two-column form, labels left, nothing boxed"),
        ],
    ),
    (
        "Calm finance",
        "A warm page, one big number, soft cards and a thin line: money explained without noise",
        "calm.example",
        &[
            ("balance", "One number, big; its caption small and grey"),
            ("activity", "Rows with air between them, amounts aligned right"),
            ("chart card", "A single thin line, no grid, no legend"),
            ("accounts", "Cards with a hairline and generous padding"),
            ("transfer", "One field at a time, the action at the bottom"),
            ("insights", "A sentence first, the chart second"),
            ("statements", "A quiet list, dates as the only structure"),
            ("onboarding", "One question per screen, large type"),
        ],
    ),
    (
        "Data dashboard",
        "Figures across the top, charts you can read in a glance, a table underneath: the numbers first",
        "charts.example",
        &[
            ("overview", "Four figures with their change, then one chart"),
            ("revenue chart", "Bars in two tones, the latest period in the lighter"),
            ("table", "Sortable columns, tabular figures, status dot at the end"),
            ("filters bar", "Date range and segments in one row above it all"),
            ("funnel", "Steps as bars of falling width, rates beside them"),
            ("cohorts", "A quiet grid, colour only where it matters"),
            ("report", "Charts in a single column, a line of reading under each"),
            ("alerts", "A short list with the number that tripped it"),
            ("detail view", "Breadcrumb, the figure, then its breakdown"),
            ("share view", "The same dashboard, read-only, with a link"),
        ],
    ),
];

/// Where the pictures of the board are kept, as the other decisions' are.
fn name(family: usize, n: usize) -> String {
    format!("taste-{family}-{n}.png")
}

/// Draw every tile, once.
pub fn make(dir: &Path) {
    if dir.join(name(2, 9)).exists() {
        return;
    }
    for (family, heights) in HEIGHTS.iter().enumerate() {
        for (n, h) in heights.iter().enumerate() {
            let picture = match family {
                0 => dense(n, *h),
                1 => calm(n, *h),
                _ => data(n, *h),
            };
            let _ = picture.save(dir.join(name(family, n)));
        }
    }
}

/// The board's question: three families, a tile for each picture drawn.
pub fn question(at: impl Fn(&str) -> String) -> Question {
    let options = FAMILIES
        .iter()
        .enumerate()
        .map(|(family, (label, line, host, tiles))| Choice {
            label: if family == 0 { format!("{label} (Recommended)") } else { (*label).into() },
            description: (*line).into(),
            tiles: tiles
                .iter()
                .enumerate()
                .map(|(n, (page, take))| Tile { image: at(&name(family, n)), source: format!("https://{host}/{page}", page = page.replace(' ', "-")), take: (*take).into(), reference: String::new() })
                .collect(),
            ..Choice::default()
        })
        .collect();
    Question { question: "Which direction should the screens take? Browse the three families and like the pictures that feel right".into(), header: "Taste".into(), options, board: true, ..Question::default() }
}
