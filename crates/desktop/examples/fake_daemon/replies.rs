//! `FAKE_REPLY_IMAGES=1`: the PM answers about the designer's drafts for
//! T-2 (a photo studio's gallery) with the drafts in its reply — a floor
//! plan, a site plan, a section and elevation — and names their folder, as
//! the person once saw it. The pictures are on "the daemon's machine": the
//! reply names them where a studio Mac keeps them (no such path here), and
//! they come only through `read_image` and `list_images`, read from
//! `$TMPDIR/equipe-fake/replies` as the daemon reads a member's. The folder
//! has two more drafts than the reply embeds. `FAKE_REPLY_IMAGES=broken`:
//! the floor plan is gone, for the failure state.
//!
//! With it, T-4 is handed in as the user saw it: three renders under
//! `outputs/` and the earth textures it changed, each in two sizes; and T-6
//! whose pictures are all gone (its worktree went, nothing kept them).

use std::path::PathBuf;

use equipe_core::protocol::rpc::{ImageList, ImageRead, ListImages, ReadImage};
use equipe_core::protocol::{Entry, Ticket};
use image::{Rgb, RgbImage};

/// Where the drafts are on the studio's Mac, as the reply names them.
pub const THERE: &str = "/Users/studio/.equipe/worktrees/photo-studio.home/t2-6ad338ba/outputs/T-2 갤러리 디자인/images";
/// T-4's worktree there.
const T4: &str = "/Users/studio/.equipe/worktrees/gallery-y/t4-6ad33ac5";

pub fn on() -> bool {
    std::env::var_os("FAKE_REPLY_IMAGES").is_some()
}

fn broken() -> bool {
    std::env::var("FAKE_REPLY_IMAGES").is_ok_and(|v| v == "broken")
}

/// Where they really are: here, for the fake.
fn dir() -> PathBuf {
    std::env::temp_dir().join("equipe-fake").join("replies")
}

fn drafts() -> PathBuf {
    dir().join("drafts")
}

fn t4() -> PathBuf {
    dir().join("t4")
}

const PAPER: Rgb<u8> = Rgb([0xFA, 0xF8, 0xF3]);
const INK: Rgb<u8> = Rgb([0x22, 0x24, 0x2A]);
const LINE: Rgb<u8> = Rgb([0x8A, 0x8F, 0x98]);
const GRID: Rgb<u8> = Rgb([0xEC, 0xE9, 0xE1]);
const ACCENT: Rgb<u8> = Rgb([0x2F, 0x6B, 0xFF]);
const GREEN: Rgb<u8> = Rgb([0x7F, 0xA8, 0x7A]);
const WOOD: Rgb<u8> = Rgb([0xD8, 0xC3, 0xA0]);

/// A sheet of drawing paper with a faint grid.
fn sheet(w: u32, h: u32) -> RgbImage {
    RgbImage::from_fn(w, h, |x, y| if x % 40 == 0 || y % 40 == 0 { GRID } else { PAPER })
}

fn rect(img: &mut RgbImage, x0: i32, y0: i32, x1: i32, y1: i32, c: Rgb<u8>) {
    for y in y0.max(0)..y1.min(img.height() as i32) {
        for x in x0.max(0)..x1.min(img.width() as i32) {
            img.put_pixel(x as u32, y as u32, c);
        }
    }
}

/// A rectangle's outline, `t` thick.
fn frame(img: &mut RgbImage, x0: i32, y0: i32, x1: i32, y1: i32, t: i32, c: Rgb<u8>) {
    rect(img, x0, y0, x1, y0 + t, c);
    rect(img, x0, y1 - t, x1, y1, c);
    rect(img, x0, y0, x0 + t, y1, c);
    rect(img, x1 - t, y0, x1, y1, c);
}

/// A line `t` thick, dashed every `dash` pixels when given.
fn line(img: &mut RgbImage, (x0, y0): (f32, f32), (x1, y1): (f32, f32), t: f32, c: Rgb<u8>, dash: Option<f32>) {
    let len = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt().max(1.);
    let steps = (len * 2.) as i32;
    for i in 0..=steps {
        let f = i as f32 / steps as f32;
        if dash.is_some_and(|d| ((f * len) / d) as i32 % 2 == 1) {
            continue;
        }
        let (x, y) = (x0 + (x1 - x0) * f, y0 + (y1 - y0) * f);
        let r = (t / 2.).max(0.5);
        rect(img, (x - r) as i32, (y - r) as i32, (x + r).ceil() as i32, (y + r).ceil() as i32, c);
    }
}

/// A circle, filled or as a ring `t` thick; `from..to` degrees of it only.
fn circle(img: &mut RgbImage, cx: f32, cy: f32, r: f32, c: Rgb<u8>, ring: Option<f32>, arc: (f32, f32)) {
    let (w, h) = (img.width() as i32, img.height() as i32);
    for y in (cy - r - 2.) as i32..(cy + r + 2.) as i32 {
        for x in (cx - r - 2.) as i32..(cx + r + 2.) as i32 {
            if x < 0 || y < 0 || x >= w || y >= h {
                continue;
            }
            let (dx, dy) = (x as f32 - cx, y as f32 - cy);
            let d = (dx * dx + dy * dy).sqrt();
            let angle = dy.atan2(dx).to_degrees().rem_euclid(360.);
            let inside = match ring {
                Some(t) => (d - r).abs() <= t / 2.,
                None => d <= r,
            };
            if inside && angle >= arc.0 && angle <= arc.1 {
                img.put_pixel(x as u32, y as u32, c);
            }
        }
    }
}

/// Diagonal hatching inside a rectangle, as a cut wall or a footprint.
fn hatch(img: &mut RgbImage, x0: i32, y0: i32, x1: i32, y1: i32, gap: i32, c: Rgb<u8>) {
    for y in y0.max(0)..y1.min(img.height() as i32) {
        for x in x0.max(0)..x1.min(img.width() as i32) {
            if (x + y).rem_euclid(gap) < 2 {
                img.put_pixel(x as u32, y as u32, c);
            }
        }
    }
}

/// A dimension line with its ticks.
fn dimension(img: &mut RgbImage, a: (f32, f32), b: (f32, f32)) {
    line(img, a, b, 2., LINE, None);
    let vertical = (a.0 - b.0).abs() < 1.;
    for p in [a, b] {
        if vertical {
            line(img, (p.0 - 10., p.1), (p.0 + 10., p.1), 2., LINE, None);
        } else {
            line(img, (p.0, p.1 - 10.), (p.0, p.1 + 10.), 2., LINE, None);
        }
    }
}

/// 평면도: a studio, a gallery hall, a lounge and a washroom, with doors and dimensions.
fn floor_plan() -> RgbImage {
    let mut img = sheet(1600, 1100);
    let (x0, y0, x1, y1) = (180, 160, 1420, 940);
    frame(&mut img, x0, y0, x1, y1, 22, INK);
    // Inner walls.
    rect(&mut img, 760, y0, 772, 640, INK);
    rect(&mut img, 772, 520, x1, 532, INK);
    rect(&mut img, 1080, 532, 1092, y1, INK);
    rect(&mut img, x0, 640, 600, 652, INK);
    // Door openings and their swings.
    for (gx, gy, r, arc) in [(760., 300., 90., (0., 90.)), (900., 520., 90., (270., 360.)), (600., 640., 80., (180., 270.)), (1080., 700., 80., (90., 180.))] {
        rect(&mut img, (gx - 2.) as i32, (gy - 2.) as i32, (gx + 14.) as i32, (gy + r) as i32, PAPER);
        circle(&mut img, gx, gy, r, LINE, Some(2.), arc);
    }
    // The gallery's hanging walls: the long wall marked in blue.
    rect(&mut img, x0 + 22, y0 + 22, 760, y0 + 32, ACCENT);
    // Furniture: a sofa, a table, the studio's backdrop.
    frame(&mut img, 260, 720, 520, 820, 4, LINE);
    circle(&mut img, 900., 760., 70., LINE, Some(4.), (0., 360.));
    rect(&mut img, 1200, 220, 1360, 236, WOOD);
    frame(&mut img, 1140, 260, 1380, 460, 3, LINE);
    // Windows on the outer wall.
    for x in [300, 520, 1000, 1240] {
        rect(&mut img, x, y0, x + 120, y0 + 22, PAPER);
        line(&mut img, (x as f32, y0 as f32 + 11.), (x as f32 + 120., y0 as f32 + 11.), 3., LINE, None);
    }
    dimension(&mut img, (x0 as f32, 1010.), (x1 as f32, 1010.));
    dimension(&mut img, (1490., y0 as f32), (1490., y1 as f32));
    // North arrow.
    circle(&mut img, 1500., 90., 34., INK, Some(3.), (0., 360.));
    line(&mut img, (1500., 120.), (1500., 60.), 4., INK, None);
    img
}

/// 배치도: the plot, the building's footprint hatched, the road, trees and parking.
fn site_plan() -> RgbImage {
    let mut img = sheet(1600, 1100);
    // The road along the bottom.
    rect(&mut img, 0, 900, 1600, 1100, Rgb([0xD9, 0xD6, 0xCF]));
    line(&mut img, (0., 1000.), (1600., 1000.), 4., PAPER, Some(40.));
    // The plot's boundary, dashed.
    let corners = [(160., 120.), (1440., 160.), (1400., 860.), (200., 860.)];
    for i in 0..4 {
        line(&mut img, corners[i], corners[(i + 1) % 4], 3., INK, Some(18.));
    }
    // The building.
    rect(&mut img, 420, 260, 1100, 640, Rgb([0xF1, 0xEE, 0xE6]));
    hatch(&mut img, 420, 260, 1100, 640, 18, LINE);
    frame(&mut img, 420, 260, 1100, 640, 8, INK);
    // The entrance canopy, in blue.
    rect(&mut img, 700, 640, 860, 700, ACCENT);
    // Parking bays.
    for n in 0..6 {
        let x = 1160 + n * 40;
        frame(&mut img, x, 660, x + 36, 820, 3, LINE);
    }
    // Trees.
    for (x, y) in [(260., 300.), (300., 520.), (260., 720.), (1260., 300.), (1320., 460.), (1200., 220.)] {
        circle(&mut img, x, y, 46., GREEN, None, (0., 360.));
        circle(&mut img, x, y, 46., Rgb([0x5E, 0x86, 0x5A]), Some(3.), (0., 360.));
    }
    circle(&mut img, 1500., 80., 34., INK, Some(3.), (0., 360.));
    line(&mut img, (1500., 110.), (1500., 50.), 4., INK, None);
    img
}

/// 단면·입면: the elevation on the left, the section on the right, on one ground line.
fn section() -> RgbImage {
    let mut img = sheet(1600, 900);
    let ground = 720;
    rect(&mut img, 0, ground, 1600, ground + 8, INK);
    hatch(&mut img, 0, ground + 8, 1600, 900, 22, LINE);
    // Elevation: the facade with a band of windows and a door.
    frame(&mut img, 120, 260, 740, ground, 8, INK);
    line(&mut img, (100., 260.), (760., 260.), 6., INK, None);
    for n in 0..4 {
        let x = 170 + n * 140;
        rect(&mut img, x, 330, x + 100, 470, Rgb([0xDD, 0xE6, 0xF7]));
        frame(&mut img, x, 330, x + 100, 470, 4, INK);
    }
    rect(&mut img, 380, 560, 480, ground, WOOD);
    frame(&mut img, 380, 560, 480, ground, 4, INK);
    // Section: the double-height gallery cut through, its slab and roof hatched.
    rect(&mut img, 860, 240, 1480, 262, INK);
    hatch(&mut img, 860, 220, 1480, 240, 10, INK);
    rect(&mut img, 860, 240, 884, ground, INK);
    rect(&mut img, 1456, 240, 1480, ground, INK);
    rect(&mut img, 1180, 470, 1480, 490, INK);
    // The skylight over the gallery, and its light in blue.
    rect(&mut img, 980, 222, 1100, 242, ACCENT);
    line(&mut img, (1040., 262.), (1000., 700.), 2., ACCENT, Some(14.));
    line(&mut img, (1040., 262.), (1110., 700.), 2., ACCENT, Some(14.));
    dimension(&mut img, (820., 262.), (820., ground as f32));
    img
}

/// A polygon filled, by its corners.
fn poly(img: &mut RgbImage, corners: &[(f32, f32)], c: Rgb<u8>) {
    let (w, h) = (img.width(), img.height());
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let mut inside = false;
            let mut j = corners.len() - 1;
            for i in 0..corners.len() {
                let ((xi, yi), (xj, yj)) = (corners[i], corners[j]);
                if (yi > py) != (yj > py) && px < (xj - xi) * (py - yi) / (yj - yi) + xi {
                    inside = !inside;
                }
                j = i;
            }
            if inside {
                img.put_pixel(x, y, c);
            }
        }
    }
}

/// 조감도 and 자재 보드: in the folder, not in the reply.
fn bird_eye() -> RgbImage {
    let mut img = sheet(1400, 1000);
    let (cx, cy, a, b, tall) = (700., 420., 360., 190., 230.);
    let (top, left, right, bottom) = ((cx, cy - b), (cx - a, cy), (cx + a, cy), (cx, cy + b));
    let down = |p: (f32, f32)| (p.0, p.1 + tall);
    poly(&mut img, &[left, bottom, down(bottom), down(left)], Rgb([0xC9, 0xC4, 0xB8]));
    poly(&mut img, &[bottom, right, down(right), down(bottom)], Rgb([0xA9, 0xA4, 0x98]));
    poly(&mut img, &[top, right, bottom, left], Rgb([0xEE, 0xEA, 0xE0]));
    // The skylight, in blue, and the edges.
    poly(&mut img, &[(cx - 40., cy - 60.), (cx + 80., cy), (cx + 40., cy + 20.), (cx - 80., cy - 40.)], ACCENT);
    for (p, q) in [(top, right), (right, bottom), (bottom, left), (left, top), (left, down(left)), (bottom, down(bottom)), (right, down(right)), (down(left), down(bottom)), (down(bottom), down(right))] {
        line(&mut img, p, q, 4., INK, None);
    }
    for (x, y) in [(330., 760.), (1080., 760.), (1180., 600.), (240., 560.)] {
        circle(&mut img, x, y, 48., GREEN, None, (0., 360.));
    }
    img
}

fn material_board() -> RgbImage {
    let mut img = RgbImage::from_pixel(1400, 1000, Rgb([0xFF, 0xFF, 0xFF]));
    let swatches = [WOOD, Rgb([0xC9, 0xC6, 0xBF]), Rgb([0x3A, 0x3D, 0x44]), Rgb([0xE9, 0xE2, 0xD3]), ACCENT, GREEN];
    for (n, c) in swatches.iter().enumerate() {
        let (x, y) = (80 + (n as i32 % 3) * 430, 80 + (n as i32 / 3) * 450);
        rect(&mut img, x, y, x + 380, y + 380, *c);
    }
    img
}

/// A render of the globe, for T-4's `outputs/`.
fn globe(hue: Rgb<u8>, w: u32, h: u32) -> RgbImage {
    let mut img = RgbImage::from_fn(w, h, |_, y| Rgb([0x0A, 0x0E, 0x1A + (y * 30 / h) as u8]));
    circle(&mut img, w as f32 / 2., h as f32 / 2., h as f32 * 0.36, hue, None, (0., 360.));
    circle(&mut img, w as f32 / 2. - 40., h as f32 / 2. - 30., h as f32 * 0.12, GREEN, None, (0., 360.));
    img
}

/// The drafts and T-4's pictures, drawn once.
pub fn make() {
    if !on() {
        return;
    }
    let drafts = drafts();
    let _ = std::fs::create_dir_all(&drafts);
    let save = |img: RgbImage, path: PathBuf| {
        if !path.exists() {
            let _ = img.save(path);
        }
    };
    if broken() {
        let _ = std::fs::remove_file(drafts.join("평면도.png"));
    } else {
        save(floor_plan(), drafts.join("평면도.png"));
    }
    save(site_plan(), drafts.join("배치도.png"));
    save(section(), drafts.join("단면·입면.png"));
    save(bird_eye(), drafts.join("조감도.png"));
    save(material_board(), drafts.join("자재 보드.png"));
    let t4 = t4();
    let _ = std::fs::create_dir_all(t4.join("outputs"));
    let _ = std::fs::create_dir_all(t4.join("public/textures/earth"));
    save(globe(Rgb([0x2F, 0x6B, 0xFF]), 1600, 1000), t4.join("outputs/T-4-globe.png"));
    save(globe(Rgb([0x3F, 0x8B, 0xEF]), 1600, 1000), t4.join("outputs/T-4-flying.png"));
    save(globe(Rgb([0x5F, 0x9B, 0xDF]), 1600, 1000), t4.join("outputs/T-4-arrived.png"));
    for (name, c) in [("earth_day", Rgb([0x4A, 0x7F, 0xB5])), ("earth_night", Rgb([0x12, 0x16, 0x30])), ("earth_bump_roughness_clouds", Rgb([0x9A, 0x9A, 0x9A]))] {
        for size in [2048, 4096] {
            let img = RgbImage::from_fn(512, 256, |x, y| if (x / 32 + y / 32) % 2 == 0 { c } else { Rgb([c[0] / 2 + 60, c[1] / 2 + 60, c[2] / 2 + 60]) });
            save(img, t4.join(format!("public/textures/earth/{name}_{size}.jpg")));
        }
    }
}

/// The PM's answer, embedding the drafts by where they are on the studio's Mac.
pub fn history() -> Vec<Entry> {
    let reply = format!(
        "디자이너가 **T-2** 갤러리 시안 세 장을 올렸어요. 전시 동선을 먼저 잡고, 촬영 스튜디오와 갤러리를 한 층에 나눴습니다.\n\n\
### 평면도\n갤러리 긴 벽(파란 선)에 작품을 걸고, 스튜디오는 북쪽 창을 씁니다.\n\n![평면도]({THERE}/평면도.png)\n\n\
### 배치도\n입구 캐노피를 도로 쪽으로 내고, 주차는 동쪽에 여섯 면입니다.\n\n![배치도]({THERE}/배치도.png \"배치도 A안\")\n\n\
### 단면·입면\n갤러리는 두 층 높이로 트고 천창으로 빛을 들입니다.\n\n![단면·입면]({THERE}/단면·입면.png)\n\n\
조감도와 자재 보드도 같은 폴더에 있어요: {THERE}/"
    );
    vec![Entry::User { text: "T-2 디자이너 시안 나왔어? 보여 줘".into() }, Entry::Reply { text: reply }]
}

/// T-4 as the user saw it, and T-6 whose pictures are all gone.
pub fn tickets(ticket: impl Fn(u32, &str, &str) -> Ticket) -> Vec<Ticket> {
    let mut t4 = ticket(4, "지구본 3D 갤러리", "외부 브랜치 equipe/frontend-t4-6ad33ac5에 남아 있습니다.");
    let at = |rel: &str| format!("{T4}/{rel}");
    t4.images = ["outputs/T-4-globe.png", "outputs/T-4-flying.png", "outputs/T-4-arrived.png"].map(at).to_vec();
    for name in ["earth_day", "earth_night", "earth_bump_roughness_clouds"] {
        for size in [4096, 2048] {
            t4.images.push(at(&format!("public/textures/earth/{name}_{size}.jpg")));
        }
    }
    let mut t6 = ticket(6, "랜딩 히어로 이미지", "히어로 이미지 세 장을 바꿨습니다.");
    t6.images = ["hero-a.png", "hero-b.png", "hero-c.png", "hero-d.png"].map(|n| format!("/Users/studio/.equipe/worktrees/gallery-y/t6-1a2b3c4d/outputs/{n}")).to_vec();
    vec![t4, t6]
}

/// Where a path the studio's Mac names is here.
fn here(path: &str) -> Option<(PathBuf, PathBuf)> {
    if let Some(rest) = path.strip_prefix(THERE) {
        return Some((drafts().join(rest.trim_start_matches('/')), drafts()));
    }
    path.strip_prefix(T4).map(|rest| (t4().join(rest.trim_start_matches('/')), t4()))
}

/// `read_image`, as the daemon on the studio's Mac answers it.
pub fn read(ask: &ReadImage) -> Result<ImageRead, String> {
    // T-6's worktree went, and its branch never had them.
    if ask.path.contains("/t6-1a2b3c4d/") {
        return Err(format!("{}: {}", ask.path, equipe_core::protocol::rpc::IMAGE_NOT_ON_BRANCH));
    }
    let (path, root) = here(&ask.path).ok_or_else(|| format!("{} isn't a picture demo shows", ask.path))?;
    equipe_core::visuals::read(&path, &[root], ask.max).map_err(|why| why.replace(&path.to_string_lossy().into_owned(), &ask.path))
}

/// `list_images`, as the daemon there answers it: the pictures in the drafts' folder.
pub fn list(ask: &ListImages) -> Result<ImageList, String> {
    let folder = ask.folder.trim_end_matches('/');
    if folder != THERE {
        return Err(format!("{folder} isn't a folder demo shows"));
    }
    let mut names: Vec<String> = std::fs::read_dir(drafts()).map_err(|e| e.to_string())?.filter_map(|e| e.ok()?.file_name().into_string().ok()).filter(|n| equipe_core::visuals::is_image_name(n)).collect();
    names.sort();
    let images: Vec<String> = names.iter().map(|n| format!("{THERE}/{n}")).collect();
    Ok(ImageList { folder: THERE.into(), total: images.len() as u32, images: images.into_iter().take(equipe_core::visuals::LIST_MAX).collect() })
}
