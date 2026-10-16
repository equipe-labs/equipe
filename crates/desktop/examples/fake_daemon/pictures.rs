//! `FAKE_DECIDE=logo`: the designer puts the logo master to the person with
//! a render of each option — the "e." with a blue round dot, the grey dot the
//! app has now (both PNG, transparent), the site's four blue dots (SVG) — and
//! has handed in T-144, the brand guide, with its pictures: a sheet of the
//! marks, the app icon, the palette. All drawn here, into
//! `$TMPDIR/equipe-fake/pictures`, and read back as the daemon reads a
//! member's pictures (`equipe_core::visuals`), from that folder only.

use std::path::{Path, PathBuf};

use equipe_core::protocol::decide::{Choice, Question, QuestionSet};
use equipe_core::protocol::rpc::{ImageRead, ReadImage};
use equipe_core::protocol::Ticket;
use image::{Rgba, RgbaImage};

/// Whether the logo decision and the brand hand-in are on.
pub fn on() -> bool {
    std::env::var("FAKE_DECIDE").is_ok_and(|v| v == "logo")
}

/// Where the pictures are drawn: the designer's worktree, as it were.
pub fn dir() -> PathBuf {
    std::env::temp_dir().join("equipe-fake").join("pictures")
}

fn at(name: &str) -> String {
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

/// The marks, the app icon and the palette, drawn once.
pub fn make() {
    let dir = dir();
    if dir.join("palette.png").exists() {
        return;
    }
    let _ = std::fs::create_dir_all(&dir);
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

/// The designer's question: which mark is the master — each option its render.
pub fn question_set(now: u64) -> QuestionSet {
    let option = |label: &str, description: &str, image: &str| Choice { label: label.into(), description: description.into(), images: vec![at(image)] };
    QuestionSet {
        id: 9003,
        session: 4,
        member: "designer".into(),
        ticket: Some(144),
        questions: vec![
            Question {
                question: "로고 마스터는 어느 안으로 할까요? (사이트는 파란 4점, 앱·스토어는 'e.' 회색 점으로 갈라져 있습니다)".into(),
                header: "로고".into(),
                options: vec![
                    option("e. + 파란 원 점 (추천)", "사이트와 앱을 하나로 — 작은 크기에서도 또렷합니다", "logo-blue-dot.png"),
                    option("e. 회색 점 유지", "지금 앱·스토어 그대로", "logo-grey-dot.png"),
                    option("파란 4점을 주 로고로", "사이트의 4점을 앱에도 씁니다", "logo-four-dots.svg"),
                ],
                ..Question::default()
            },
            Question {
                question: "앱 아이콘 배경은 어떻게 할까요?".into(),
                header: "아이콘".into(),
                options: vec![option("파란 바탕 (추천)", "스토어에서 눈에 띕니다", "app-icon.png"), Choice { label: "흰 바탕".into(), description: "시안은 다음에 그려 드릴게요".into(), ..Choice::default() }],
                ..Question::default()
            },
        ],
        asked_at: now.saturating_sub(300),
    }
}

/// T-144, handed in with its pictures.
pub fn hand_in(mut ticket: Ticket) -> Ticket {
    ticket.kind = equipe_core::protocol::TicketKind::Task;
    ticket.images = ["brand-sheet.png", "logo-blue-dot.png", "app-icon.png", "palette.png"].map(at).to_vec();
    ticket
}

/// The news of it in the PM's conversation, as the daemon words it.
pub const NEWS: &str = "designer handed in T-144 for your review · equipe 브랜드 정리·체계화 (이름·로고 유지).";

/// A picture read back: from the pictures' folder only, as the daemon reads a member's.
pub fn read(ask: &ReadImage) -> Result<ImageRead, String> {
    equipe_core::visuals::read(Path::new(&ask.path), &[dir()], ask.max)
}
