//! `FAKE_DECIDE=logo`: the designer puts the logo master to the person with
//! a render of each option — the "e." with a blue round dot, the grey dot the
//! app has now (both PNG, transparent), the site's four blue dots (SVG) — and
//! has handed in T-144, the brand guide, with its pictures: a sheet of the
//! marks, the app icon, the palette. All drawn here, into
//! `$TMPDIR/equipe-fake/pictures`, and read back as the daemon reads a
//! member's pictures (`equipe_core::visuals`), from that folder only.
//!
//! `FAKE_DECIDE=brand`: the designer's three brand directions instead, each
//! drawn on the product's home screen (calm, bold, warm), every option with
//! the aspects the person can mix — type, colour, shape.
//!
//! `FAKE_DECIDE_CASE=<case>[,<case>…]` puts other logo questions to the
//! person instead, one a step, for the grid's edge cases: `four` (the Gallery
//! Y question — a picture of its own over four options, as a member once
//! asked it), `five`, `eight` (several-pick), `mixed` (some options without a
//! picture, long and mixed-language labels, emoji), `aspect` (a panorama, a
//! tower, a square, an SVG) and `lead` (a picture of its own over options
//! that have none).

use std::path::{Path, PathBuf};

use equipe_core::protocol::decide::{Choice, Question, QuestionSet};
use equipe_core::protocol::rpc::{ImageRead, ReadImage};
use equipe_core::protocol::Ticket;
use image::{Rgba, RgbaImage};

/// Whether the logo decision (or the brand directions) and the brand hand-in are on.
pub fn on() -> bool {
    std::env::var("FAKE_DECIDE").is_ok_and(|v| v == "logo" || v == "brand")
}

/// Whether the brand directions are asked, rather than the logo.
fn brand() -> bool {
    std::env::var("FAKE_DECIDE").is_ok_and(|v| v == "brand")
}

/// Where the pictures are drawn: the designer's worktree, as it were.
pub fn dir() -> PathBuf {
    std::env::temp_dir().join("equipe-fake").join("pictures")
}

pub fn at(name: &str) -> String {
    let dir = dir();
    let dir = dir.canonicalize().unwrap_or(dir);
    dir.join(name).to_string_lossy().into_owned()
}

const INK: [u8; 4] = [0x16, 0x18, 0x1D, 0xFF];
const BLUE: [u8; 4] = [0x2F, 0x6B, 0xFF, 0xFF];
const GREY: [u8; 4] = [0x9A, 0xA0, 0xA6, 0xFF];

/// A picture drawn by `paint` (what colour, if any, a point of the unit
/// square is), four samples a pixel so its edges are smooth.
fn draw(w: u32, h: u32, background: Option<[u8; 4]>, paint: impl Fn(f32, f32) -> Option<[u8; 4]>) -> RgbaImage {
    RgbaImage::from_fn(w, h, |x, y| {
        let mut sum = [0f32; 4];
        let mut hits = 0.;
        for (dx, dy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
            let (u, v) = ((x as f32 + dx) / w as f32, (y as f32 + dy) / h as f32);
            if let Some(c) = paint(u, v).or(background) {
                for i in 0..4 {
                    sum[i] += c[i] as f32 * if i == 3 { 1. } else { c[3] as f32 / 255. };
                }
                hits += 1.;
            }
        }
        if hits == 0. {
            return Rgba([0, 0, 0, 0]);
        }
        let alpha = sum[3] / 4.;
        let colour = |i: usize| if alpha > 0. { (sum[i] / 4. / (alpha / 255.)).min(255.) as u8 } else { 0 };
        Rgba([colour(0), colour(1), colour(2), alpha as u8])
    })
}

/// The "e": a ring open at its lower right, with its bar — at `(cx, cy)`,
/// `r` across, in the unit square.
fn e(u: f32, v: f32, cx: f32, cy: f32, r: f32) -> bool {
    let (x, y) = (u - cx, v - cy);
    let d = (x * x + y * y).sqrt();
    let ring = d <= r && d >= r * 0.64;
    let angle = y.atan2(x).to_degrees();
    let open = (8.0..58.).contains(&angle);
    let bar = x >= -r * 0.7 && x <= r && (-0.06 * r..=0.13 * r).contains(&(y + 0.04 * r));
    (ring && !open) || (bar && d <= r)
}

fn dot(u: f32, v: f32, cx: f32, cy: f32, r: f32) -> bool {
    (u - cx).powi(2) + (v - cy).powi(2) <= r * r
}

/// The pictures of the edge cases' options: a few plain marks, each its own,
/// and the extreme shapes — drawn once.
fn make_cases(dir: &Path) {
    if dir.join("case-tower.png").exists() {
        return;
    }
    let wall = |n: f32| move |u: f32, v: f32| {
        let bars = (0..3).any(|i| {
            let x = 0.22 + i as f32 * 0.22 + n * 0.02;
            (x..x + 0.1).contains(&u) && (0.2 + i as f32 * 0.08..0.82).contains(&v)
        });
        bars.then_some(INK)
    };
    let _ = draw(800, 600, None, wall(0.)).save(dir.join("case-walls.png"));
    // A cross of light through a dark wall.
    let _ = draw(800, 600, Some([0x26, 0x24, 0x22, 0xFF]), |u, v| (((0.47..0.53).contains(&u) && (0.12..0.88).contains(&v)) || ((0.3..0.7).contains(&u) && (0.36..0.42).contains(&v))).then_some([0xFF, 0xF4, 0xD8, 0xFF])).save(dir.join("case-light.png"));
    let _ = draw(800, 600, None, |u, v| {
        let ring = ((u - 0.5) * 1.33).hypot(v - 0.5);
        ((0.22..0.3).contains(&ring) || ((0.47..0.53).contains(&u) && (0.2..0.8).contains(&v))).then_some(BLUE)
    })
    .save(dir.join("case-tie.png"));
    let _ = draw(800, 600, None, |u, v| if dot(u * 1.33, v, 0.5, 0.5, 0.18) { Some(BLUE) } else if (0.56..0.82).contains(&u) && (0.32..0.68).contains(&v) { Some(INK) } else { None }).save(dir.join("case-circle-square.png"));
    let hues: [[u8; 4]; 4] = [[0xFF, 0x6B, 0x5A, 0xFF], [0x2C, 0xB6, 0x7D, 0xFF], [0xFF, 0xB0, 0x20, 0xFF], [0x8B, 0x5C, 0xF6, 0xFF]];
    for (n, hue) in hues.iter().enumerate() {
        let hue = *hue;
        let _ = draw(800, 600, None, move |u, v| (dot(u * 1.33, v, 0.42 + n as f32 * 0.1, 0.5, 0.2) || ((0.18..0.82).contains(&u) && (0.78..0.84).contains(&v))).then_some(hue)).save(dir.join(format!("case-mark-{n}.png")));
    }
    // A panorama ten times its height, and a tower ten times its width.
    let _ = draw(2400, 240, Some([0xF4, 0xF5, 0xF7, 0xFF]), |u, v| ((u * 12.).fract() < 0.5 && v > 0.3).then_some(BLUE)).save(dir.join("case-wide.png"));
    let _ = draw(240, 2400, Some([0x16, 0x18, 0x1D, 0xFF]), |u, v| ((v * 12.).fract() < 0.5 && u > 0.3).then_some([0xFF, 0xB0, 0x20, 0xFF])).save(dir.join("case-tower.png"));
    let _ = draw(600, 600, None, |u, v| dot(u, v, 0.5, 0.5, 0.4).then_some(INK)).save(dir.join("case-square.png"));
    // What the site has now, the question's own.
    let _ = draw(1200, 800, Some([0xFF, 0xFF, 0xFF, 0xFF]), |u, v| if (0.06..0.94).contains(&u) && (0.08..0.2).contains(&v) { Some([0xEE, 0xEF, 0xF2, 0xFF]) } else if e(u * 1.5, v, 0.3, 0.55, 0.16) { Some(INK) } else { None }).save(dir.join("case-now.png"));
}

/// A direction's tokens, as its home screen shows them.
struct Tokens {
    bg: [u8; 4],
    surface: [u8; 4],
    ink: [u8; 4],
    muted: [u8; 4],
    accent: [u8; 4],
    on_accent: [u8; 4],
    /// Corner radius, in the screen's height.
    radius: f32,
    /// How thick the headline's strokes are, in the screen's height.
    weight: f32,
}

/// Whether `(x, y)` — x in the screen's height too — is in the box from
/// `(x0, y0)` to `(x1, y1)` with corners `r` round.
fn rounded(x: f32, y: f32, (x0, y0, x1, y1): (f32, f32, f32, f32), r: f32) -> bool {
    if x < x0 || x > x1 || y < y0 || y > y1 {
        return false;
    }
    let r = r.min((x1 - x0) / 2.).min((y1 - y0) / 2.);
    let cx = x.max(x0 + r).min((x1 - r).max(x0 + r));
    let cy = y.max(y0 + r).min((y1 - r).max(y0 + r));
    (x - cx).powi(2) + (y - cy).powi(2) <= r * r
}

/// A line of words at `(x, y)`, `w` long and `h` high: blocks a word each,
/// rounded as the type is.
fn words(x: f32, y: f32, at: (f32, f32), w: f32, h: f32, round: f32) -> bool {
    let (x0, y0) = at;
    if y < y0 || y > y0 + h || x < x0 || x > x0 + w {
        return false;
    }
    // Words of a few widths, a gap between.
    let lengths = [0.09, 0.05, 0.13, 0.07, 0.1, 0.06, 0.11];
    let mut start = x0;
    for (i, len) in lengths.iter().cycle().enumerate().take(40) {
        let len = len * (h / 0.04).clamp(0.6, 1.4);
        let end = (start + len).min(x0 + w);
        if x >= start && x <= end {
            return rounded(x, y, (start, y0, end, y0 + h), h * round);
        }
        start = end + h * 0.45 + (i % 2) as f32 * 0.004;
        if start >= x0 + w {
            break;
        }
    }
    false
}

/// The product's home screen in a direction's tokens: its bar, a headline
/// and a line under it, the call to action and a quieter button, a picture,
/// and three cards.
fn home(t: &Tokens) -> impl Fn(f32, f32) -> Option<[u8; 4]> + '_ {
    const WIDE: f32 = 4. / 3.;
    move |u, v| {
        let x = u * WIDE;
        let r = t.radius;
        let round = if r > 0.03 { 0.5 } else if r > 0. { 0.25 } else { 0. };
        // The bar: a mark, three links, a button.
        if v < 0.1 {
            if (x - 0.07).powi(2) + (v - 0.05).powi(2) <= 0.018f32.powi(2) {
                return Some(t.accent);
            }
            if words(x, v, (0.1, 0.04), 0.12, 0.022, round) {
                return Some(t.ink);
            }
            for (i, at) in [0.72, 0.84, 0.96].iter().enumerate() {
                if words(x, v, (*at, 0.042), 0.07 + i as f32 * 0.005, 0.016, round) {
                    return Some(t.muted);
                }
            }
            if rounded(x, v, (1.1, 0.025, 1.27, 0.075), r * 0.6) {
                return Some(if words(x, v, (1.13, 0.043), 0.11, 0.014, round) { t.on_accent } else { t.ink });
            }
            return Some(t.surface);
        }
        if (0.1..0.103).contains(&v) {
            return Some(t.muted);
        }
        // The headline, its line, the buttons.
        let head = t.weight;
        if words(x, v, (0.07, 0.17), 0.56, head, round) || words(x, v, (0.07, 0.17 + head * 1.45), 0.38, head, round) {
            return Some(t.ink);
        }
        let below = 0.17 + head * 2.9 + 0.02;
        if words(x, v, (0.07, below), 0.5, 0.018, round) || words(x, v, (0.07, below + 0.034), 0.42, 0.018, round) {
            return Some(t.muted);
        }
        let buttons = below + 0.09;
        if rounded(x, v, (0.07, buttons, 0.29, buttons + 0.07), r) {
            return Some(if words(x, v, (0.1, buttons + 0.027), 0.15, 0.017, round) { t.on_accent } else { t.accent });
        }
        if rounded(x, v, (0.31, buttons, 0.5, buttons + 0.07), r) {
            let inside = rounded(x, v, (0.313, buttons + 0.003, 0.497, buttons + 0.067), r);
            return Some(if !inside || words(x, v, (0.34, buttons + 0.027), 0.12, 0.017, round) { t.ink } else { t.bg });
        }
        // The picture: the accent, lit from its corner.
        if rounded(x, v, (0.72, 0.16, 1.27, 0.52), r * 1.4) {
            let light = ((x - 0.72) / 0.55 * 0.5 + (v - 0.16) / 0.36 * 0.5).clamp(0., 1.);
            let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * light * 0.55) as u8;
            let disc = (x - 1.02).powi(2) + (v - 0.33).powi(2) <= 0.09f32.powi(2);
            let base = if disc { t.on_accent } else { t.accent };
            return Some([mix(base[0], t.surface[0]), mix(base[1], t.surface[1]), mix(base[2], t.surface[2]), 0xFF]);
        }
        // Three cards.
        for i in 0..3 {
            let x0 = 0.07 + i as f32 * 0.405;
            let card = (x0, 0.6, x0 + 0.38, 0.94);
            if rounded(x, v, card, r) {
                if rounded(x, v, (x0 + 0.02, 0.62, x0 + 0.36, 0.77), r * 0.7) {
                    let tint = [0.18, 0.3, 0.42][i];
                    let m = |a: u8, b: u8| (a as f32 * tint + b as f32 * (1. - tint)) as u8;
                    return Some([m(t.accent[0], t.bg[0]), m(t.accent[1], t.bg[1]), m(t.accent[2], t.bg[2]), 0xFF]);
                }
                if words(x, v, (x0 + 0.025, 0.8), 0.26, 0.022, round) {
                    return Some(t.ink);
                }
                if words(x, v, (x0 + 0.025, 0.845), 0.3, 0.015, round) || words(x, v, (x0 + 0.025, 0.875), 0.2, 0.015, round) {
                    return Some(t.muted);
                }
                return Some(t.surface);
            }
        }
        Some(t.bg)
    }
}

/// The three directions' home screens, drawn once.
fn make_brand(dir: &Path) {
    if dir.join("brand-warm.png").exists() {
        return;
    }
    let calm = Tokens {
        bg: [0xF6, 0xF6, 0xF4, 0xFF],
        surface: [0xFF, 0xFF, 0xFF, 0xFF],
        ink: [0x1C, 0x1D, 0x21, 0xFF],
        muted: [0xA3, 0xA7, 0xAE, 0xFF],
        accent: [0x3A, 0x5B, 0xD9, 0xFF],
        on_accent: [0xFF, 0xFF, 0xFF, 0xFF],
        radius: 0.018,
        weight: 0.032,
    };
    let bold = Tokens {
        bg: [0x10, 0x12, 0x14, 0xFF],
        surface: [0x1C, 0x1E, 0x22, 0xFF],
        ink: [0xF4, 0xF4, 0xF2, 0xFF],
        muted: [0x6E, 0x73, 0x7B, 0xFF],
        accent: [0xFF, 0x5A, 0x1F, 0xFF],
        on_accent: [0x10, 0x12, 0x14, 0xFF],
        radius: 0.,
        weight: 0.056,
    };
    let warm = Tokens {
        bg: [0xF3, 0xF5, 0xF0, 0xFF],
        surface: [0xFF, 0xFF, 0xFC, 0xFF],
        ink: [0x22, 0x33, 0x2A, 0xFF],
        muted: [0x93, 0xA0, 0x97, 0xFF],
        accent: [0x2E, 0x8B, 0x57, 0xFF],
        on_accent: [0xFF, 0xFF, 0xFF, 0xFF],
        radius: 0.05,
        weight: 0.042,
    };
    for (name, tokens) in [("brand-calm.png", calm), ("brand-bold.png", bold), ("brand-warm.png", warm)] {
        let _ = draw(800, 600, None, home(&tokens)).save(dir.join(name));
    }
}

/// The marks, the app icon and the palette, drawn once.
pub fn make() {
    let dir = dir();
    let _ = std::fs::create_dir_all(&dir);
    make_cases(&dir);
    make_brand(&dir);
    if dir.join("palette.png").exists() {
        return;
    }
    let mark = |dot_colour: [u8; 4]| move |u: f32, v: f32| if e(u, v, 0.42, 0.52, 0.27) { Some(INK) } else if dot(u, v, 0.79, 0.74, 0.075) { Some(dot_colour) } else { None };
    let _ = draw(960, 960, None, mark(BLUE)).save(dir.join("logo-blue-dot.png"));
    let _ = draw(960, 960, None, mark(GREY)).save(dir.join("logo-grey-dot.png"));
    let four = r##"<svg xmlns="http://www.w3.org/2000/svg" width="960" height="960" viewBox="0 0 96 96">
<circle cx="34" cy="34" r="11" fill="#2F6BFF"/><circle cx="62" cy="34" r="11" fill="#2F6BFF"/>
<circle cx="34" cy="62" r="11" fill="#2F6BFF"/><circle cx="62" cy="62" r="11" fill="#7FA6FF"/>
</svg>"##;
    let _ = std::fs::write(dir.join("logo-four-dots.svg"), four);
    // The brand sheet: the mark on white and on ink, side by side.
    let sheet = draw(1600, 900, Some([0xF6, 0xF6, 0xF8, 0xFF]), |u, v| {
        let left = u < 0.5;
        let (lu, lv) = (if left { u * 2. } else { (u - 0.5) * 2. }, v);
        let inside = (0.06..0.94).contains(&lu) && (0.1..0.9).contains(&lv);
        let card = if left { [0xFF, 0xFF, 0xFF, 0xFF] } else { [0x16, 0x18, 0x1D, 0xFF] };
        let ink = if left { INK } else { [0xF4, 0xF5, 0xF7, 0xFF] };
        if !inside {
            return None;
        }
        // Each card is about square (704 × 720 pixels): the mark in it as drawn.
        let (su, sv) = ((lu - 0.06) / 0.88, (lv - 0.1) / 0.8);
        if e(su, sv, 0.44, 0.52, 0.22) {
            Some(ink)
        } else if dot(su, sv, 0.74, 0.7, 0.06) {
            Some(BLUE)
        } else {
            Some(card)
        }
    });
    let _ = sheet.save(dir.join("brand-sheet.png"));
    // The app icon: the mark in white on the blue, rounded.
    let icon = draw(1024, 1024, None, |u, v| {
        let (x, y) = ((u - 0.5).abs(), (v - 0.5).abs());
        let corner = 0.22;
        let inside = x <= 0.5 && y <= 0.5 && ((x <= 0.5 - corner || y <= 0.5 - corner) || (x - (0.5 - corner)).powi(2) + (y - (0.5 - corner)).powi(2) <= corner * corner);
        if !inside {
            None
        } else if e(u, v, 0.45, 0.52, 0.24) {
            Some([0xFF, 0xFF, 0xFF, 0xFF])
        } else if dot(u, v, 0.76, 0.72, 0.065) {
            Some([0xBF, 0xD3, 0xFF, 0xFF])
        } else {
            Some(BLUE)
        }
    });
    let _ = icon.save(dir.join("app-icon.png"));
    // The palette: five swatches.
    let swatches = [[0x2F, 0x6B, 0xFF, 0xFF], [0x16, 0x18, 0x1D, 0xFF], [0x9A, 0xA0, 0xA6, 0xFF], [0xF4, 0xF5, 0xF7, 0xFF], [0xFF, 0xB0, 0x20, 0xFF]];
    let palette = draw(1500, 600, Some([0xFF, 0xFF, 0xFF, 0xFF]), |u, v| {
        let n = (u * 5.).floor().min(4.) as usize;
        let cell = u * 5. - n as f32;
        ((0.08..0.92).contains(&cell) && (0.12..0.88).contains(&v)).then_some(swatches[n])
    });
    let _ = palette.save(dir.join("palette.png"));
}

/// One of `FAKE_DECIDE_CASE`'s questions.
fn case(name: &str) -> Option<Question> {
    let option = |label: &str, description: &str, image: &str| Choice { label: label.into(), description: description.into(), images: if image.is_empty() { Vec::new() } else { vec![at(image)] }, ..Choice::default() };
    let marks = |n: usize| (0..n).map(|i| option(&format!("Option {}", (b'A' + i as u8) as char), "Corners a little rounder — reads at small sizes too", &format!("case-mark-{}.png", i % 4))).collect::<Vec<_>>();
    Some(match name {
        "four" => Question {
            question: "Which option should be the Gallery Y logo?".into(),
            header: "Logo".into(),
            images: vec![at("case-now.png")],
            options: vec![
                option("A Three walls (Recommended)", "Three walls taking in the light — reads as the same shape anywhere, down to a 16px favicon", "case-walls.png"),
                option("B Church of the Light", "A cross of light splitting the wall — Tadao Ando's architectural language. It may read as strongly religious, though", "case-light.png"),
                option("C Form ties", "The form-tie holes of concrete formwork — the material's traces used as they are", "case-tie.png"),
                option("D Circle and square", "A circle against a square — the simplest, but it may overlap with other galleries", "case-circle-square.png"),
            ],
            ..Question::default()
        },
        "five" => Question { question: "Which of the five shall we go with?".into(), header: "Logo".into(), options: marks(5), ..Question::default() },
        "eight" => Question { question: "Pick every mark for the sticker set".into(), header: "Stickers".into(), options: marks(8), multi_select: true, ..Question::default() },
        "mixed" => Question {
            question: "What should the app icon's background be?".into(),
            header: "Icon".into(),
            options: vec![
                option("Blue background (Recommended)", "Stands out in the store", "app-icon.png"),
                option("White background", "I'll draw this one next time", ""),
                option("🎨 Gradient — a gradient background running from blue into purple, with a very long name", "A long English description that keeps going, 日本語も混ざって (wide letters mixed in), past two lines of the tile so it has to end in an ellipsis somewhere", "case-mark-3.png"),
                option("Dark mode only", "", ""),
                option("Circle and square", "A circle against a square", "case-circle-square.png"),
            ],
            ..Question::default()
        },
        "aspect" => Question {
            question: "Which banner shape works best?".into(),
            header: "Banner".into(),
            options: vec![option("A wide strip", "2400×240", "case-wide.png"), option("A tall tower", "240×2400", "case-tower.png"), option("Square", "600×600", "case-square.png"), option("SVG four dots", "An SVG that gives no size", "logo-four-dots.svg")],
            ..Question::default()
        },
        "lead" => Question {
            question: "Keep the site's header as it is?".into(),
            header: "Header".into(),
            images: vec![at("case-now.png")],
            options: vec![Choice { label: "As it is (Recommended)".into(), description: "Not touched in this release".into(), ..Choice::default() }, Choice { label: "Change the logo only".into(), description: "Swap in the new mark, leave the rest".into(), ..Choice::default() }],
            ..Question::default()
        },
        _ => return None,
    })
}

/// The designer's question: which mark is the master — each option its render.
/// With `FAKE_DECIDE_CASE`, the edge cases' questions instead.
pub fn question_set(now: u64) -> QuestionSet {
    let option = |label: &str, description: &str, image: &str| Choice { label: label.into(), description: description.into(), images: vec![at(image)], ..Choice::default() };
    let aspects = |names: &[&str]| names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>();
    if brand() {
        let direction = |label: &str, description: &str, image: &str| Choice { aspects: aspects(&["Type", "Colour", "Shape"]), ..option(label, description, image) };
        return QuestionSet {
            id: 9003,
            session: 4,
            member: "designer".into(),
            ticket: Some(150),
            questions: vec![Question {
                question: "Which way should the brand go? I put all three on the home screen".into(),
                header: "Brand".into(),
                options: vec![
                    direction("Calm (Recommended)", "Light type, generous space and a single blue — easy on the eyes for hours", "brand-calm.png"),
                    direction("Bold", "Heavy type on a dark background, square corners — remembered at first sight", "brand-bold.png"),
                    direction("Warm", "Round corners and green — feels approachable", "brand-warm.png"),
                ],
                ..Question::default()
            }],
            asked_at: now.saturating_sub(120),
            review: false,
            retro: None,
        };
    }
    if let Some(cases) = std::env::var("FAKE_DECIDE_CASE").ok().filter(|c| !c.is_empty()) {
        let questions: Vec<Question> = cases.split(',').filter_map(|c| case(c.trim())).collect();
        if !questions.is_empty() {
            return QuestionSet { id: 9003, session: 4, member: "designer".into(), ticket: Some(24), questions, asked_at: now.saturating_sub(300), review: false, retro: None };
        }
    }
    QuestionSet {
        id: 9003,
        session: 4,
        member: "designer".into(),
        ticket: Some(144),
        questions: vec![
            Question {
                question: "Which option should be the master logo? (The site uses four blue dots, the app and stores an 'e.' with a grey dot)".into(),
                header: "Logo".into(),
                options: vec![
                    Choice { aspects: aspects(&["Dot"]), ..option("e. + a round blue dot (Recommended)", "One mark for site and app — crisp at small sizes", "logo-blue-dot.png") },
                    Choice { aspects: aspects(&["Dot"]), ..option("Keep the e. grey dot", "The app and stores as they are", "logo-grey-dot.png") },
                    Choice { aspects: aspects(&["Colour"]), ..option("Four blue dots as the main logo", "The site's four dots in the app too", "logo-four-dots.svg") },
                ],
                ..Question::default()
            },
            Question {
                question: "What should the app icon's background be?".into(),
                header: "Icon".into(),
                options: vec![option("Blue background (Recommended)", "Stands out in the store", "app-icon.png"), Choice { label: "White background".into(), description: "I'll draw this one next time".into(), ..Choice::default() }],
                ..Question::default()
            },
        ],
        asked_at: now.saturating_sub(300),
        review: false,
        retro: None,
    }
}

/// T-144, handed in with its pictures.
pub fn hand_in(mut ticket: Ticket) -> Ticket {
    ticket.kind = equipe_core::protocol::TicketKind::Task;
    ticket.images = ["brand-sheet.png", "logo-blue-dot.png", "app-icon.png", "palette.png"].map(at).to_vec();
    ticket
}

/// The news of it in the PM's conversation, as the daemon words it.
pub const NEWS: &str = "designer handed in T-144 for your review · equipe brand clean-up and system (name and logo kept).";

/// A picture read back: from the pictures' folder only, as the daemon reads a member's.
pub fn read(ask: &ReadImage) -> Result<ImageRead, String> {
    equipe_core::visuals::read(Path::new(&ask.path), &[dir()], ask.max)
}
