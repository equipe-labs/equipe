//! `FAKE_DECIDE=logo`: the designer puts the logo master to the person with
//! a render of each option — the "e." with a blue round dot, the grey dot the
//! app has now (both PNG, transparent), the site's four blue dots (SVG) — and
//! has handed in T-144, the brand guide, with its pictures: a sheet of the
//! marks, the app icon, the palette. All drawn here, into
//! `$TMPDIR/equipe-fake/pictures`, and read back as the daemon reads a
//! member's pictures (`equipe_core::visuals`), from that folder only.
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

/// The marks, the app icon and the palette, drawn once.
pub fn make() {
    let dir = dir();
    let _ = std::fs::create_dir_all(&dir);
    make_cases(&dir);
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
    let option = |label: &str, description: &str, image: &str| Choice { label: label.into(), description: description.into(), images: if image.is_empty() { Vec::new() } else { vec![at(image)] } };
    let marks = |n: usize| (0..n).map(|i| option(&format!("{} 안", (b'A' + i as u8) as char), "모서리를 조금 더 둥글게 — 작은 크기에서도 읽힙니다", &format!("case-mark-{}.png", i % 4))).collect::<Vec<_>>();
    Some(match name {
        "four" => Question {
            question: "Gallery Y 로고로 어느 안을 쓸까요?".into(),
            header: "로고".into(),
            images: vec![at("case-now.png")],
            options: vec![
                option("A 세 벽 (Recommended)", "세 개의 벽이 빛을 받아들이는 공간 — 16px 파비콘 어디서나 같은 모양으로 읽힙니다", "case-walls.png"),
                option("B 빛의 교회", "벽을 가르는 십자형 빛 — 안도 다다오의 건축 언어예요. 다만 종교적인 연상이 강할 수 있습니다", "case-light.png"),
                option("C 폼타이", "콘크리트 거푸집의 폼타이 구멍 — 재료의 흔적을 그대로 씁니다", "case-tie.png"),
                option("D 원과 사각", "원과 사각의 대비 — 가장 단순하지만 다른 갤러리와 겹칠 수 있습니다", "case-circle-square.png"),
            ],
            ..Question::default()
        },
        "five" => Question { question: "다섯 안 가운데 어느 것으로 갈까요?".into(), header: "로고".into(), options: marks(5), ..Question::default() },
        "eight" => Question { question: "스티커 세트에 넣을 마크를 모두 골라 주세요".into(), header: "스티커".into(), options: marks(8), multi_select: true, ..Question::default() },
        "mixed" => Question {
            question: "앱 아이콘 배경은 어떻게 할까요?".into(),
            header: "아이콘".into(),
            options: vec![
                option("파란 바탕 (추천)", "스토어에서 눈에 띕니다", "app-icon.png"),
                option("흰 바탕", "시안은 다음에 그려 드릴게요", ""),
                option("🎨 Gradient — 파랑에서 보라로 이어지는 아주 긴 이름의 그라데이션 배경 안", "A long English description that keeps going, 한국어와 섞여서, past two lines of the tile so it has to end in an ellipsis somewhere", "case-mark-3.png"),
                option("Dark 모드 전용", "", ""),
                option("원과 사각", "원과 사각의 대비", "case-circle-square.png"),
            ],
            ..Question::default()
        },
        "aspect" => Question {
            question: "배너 모양은 어느 쪽이 좋을까요?".into(),
            header: "배너".into(),
            options: vec![option("가로로 긴 띠", "2400×240", "case-wide.png"), option("세로로 긴 탑", "240×2400", "case-tower.png"), option("정사각", "600×600", "case-square.png"), option("SVG 4점", "크기를 말하지 않는 SVG", "logo-four-dots.svg")],
            ..Question::default()
        },
        "lead" => Question {
            question: "지금 사이트 머리글을 그대로 둘까요?".into(),
            header: "머리글".into(),
            images: vec![at("case-now.png")],
            options: vec![Choice { label: "그대로 (추천)".into(), description: "이번 릴리스에서는 손대지 않습니다".into(), ..Choice::default() }, Choice { label: "로고만 바꾸기".into(), description: "새 마크로 바꾸고 나머지는 둡니다".into(), ..Choice::default() }],
            ..Question::default()
        },
        _ => return None,
    })
}

/// The designer's question: which mark is the master — each option its render.
/// With `FAKE_DECIDE_CASE`, the edge cases' questions instead.
pub fn question_set(now: u64) -> QuestionSet {
    let option = |label: &str, description: &str, image: &str| Choice { label: label.into(), description: description.into(), images: vec![at(image)] };
    if let Some(cases) = std::env::var("FAKE_DECIDE_CASE").ok().filter(|c| !c.is_empty()) {
        let questions: Vec<Question> = cases.split(',').filter_map(|c| case(c.trim())).collect();
        if !questions.is_empty() {
            return QuestionSet { id: 9003, session: 4, member: "designer".into(), ticket: Some(24), questions, asked_at: now.saturating_sub(300) };
        }
    }
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
