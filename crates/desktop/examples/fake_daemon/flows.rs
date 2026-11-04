//! `FAKE_DESIGN=1`'s UI map: the web project's flows — 온보딩, 결제, 검색,
//! 설정 — their screens in every state, light and dark at 390 and 1280,
//! some states missing and one screen stale; the pictures drawn as small
//! placeholder PNGs under the web project's `design/flows/`, as the
//! designer keeps them. 온보딩 has a walkthrough (an MP4 made with
//! `ffmpeg` when this Mac has it; else its path is absent, as one on another
//! machine) and an older one; 결제's is on "another machine"; 검색 has none.
//! No `flows.json` is written: the map is put on the system as the daemon
//! would read it, so its counts and marks are exactly these.

use std::path::{Path, PathBuf};

use equipe_core::protocol::design::{DesignSystem, Flow, FlowMap, Mark, Picture, Screen, ScreenState, Walkthrough};

/// How a state is pictured: not at all, light and dark at 390, or at 390 and 1280.
#[derive(Clone, Copy, PartialEq)]
enum Drawn {
    No,
    Phone,
    Both,
}

struct S {
    id: &'static str,
    title: &'static str,
    note: Option<&'static str>,
    ticket: Option<u32>,
    states: &'static [(&'static str, &'static str, &'static str, Option<&'static str>, Drawn)],
    missing: &'static [&'static str],
    stale: bool,
}

struct F {
    id: &'static str,
    title: &'static str,
    summary: &'static str,
    screens: &'static [S],
}

const FLOWS: &[F] = &[
    F {
        id: "onboarding",
        title: "온보딩",
        summary: "첫 방문부터 첫 프로젝트까지",
        screens: &[
            S {
                id: "login",
                title: "로그인",
                note: Some("이메일과 비밀번호. 틀리면 필드 아래 한 줄로 알려요."),
                ticket: Some(142),
                states: &[("default", "기본", "default", None, Drawn::Both), ("error", "오류", "error", Some("비밀번호가 틀렸을 때"), Drawn::Both), ("loading", "로딩", "loading", None, Drawn::Phone)],
                missing: &["offline"],
                stale: false,
            },
            S {
                id: "signup",
                title: "가입",
                note: Some("이름·이메일·비밀번호 세 칸, 약관은 한 줄."),
                ticket: Some(143),
                states: &[("default", "기본", "default", None, Drawn::Both), ("error", "오류", "error", Some("이미 쓰는 이메일일 때"), Drawn::Phone), ("long", "긴 글", "long", Some("이름이 아주 길 때"), Drawn::Phone)],
                missing: &[],
                stale: true,
            },
            S {
                id: "welcome",
                title: "첫 프로젝트",
                note: None,
                ticket: None,
                states: &[("first", "첫 사용", "first", Some("처음 들어왔을 때 — 안내 세 줄"), Drawn::Both), ("default", "기본", "default", None, Drawn::Both), ("empty", "빈 상태", "empty", Some("프로젝트를 모두 지웠을 때"), Drawn::No)],
                missing: &["empty"],
                stale: false,
            },
        ],
    },
    F {
        id: "checkout",
        title: "결제",
        summary: "요금제 고르기부터 영수증까지",
        screens: &[
            S {
                id: "plans",
                title: "요금제",
                note: Some("월·연 전환, 추천 요금제 하나만 강조."),
                ticket: Some(151),
                states: &[("default", "기본", "default", None, Drawn::Both), ("permission", "권한 없음", "permission", Some("팀 관리자가 아닐 때"), Drawn::Phone)],
                missing: &[],
                stale: false,
            },
            S {
                id: "pay",
                title: "결제 정보",
                note: None,
                ticket: Some(152),
                states: &[("default", "기본", "default", None, Drawn::Both), ("error", "오류", "error", Some("카드가 거절됐을 때"), Drawn::Both), ("loading", "로딩", "loading", None, Drawn::Phone)],
                missing: &["offline"],
                stale: false,
            },
            S { id: "receipt", title: "영수증", note: None, ticket: None, states: &[("success", "성공", "success", None, Drawn::Both)], missing: &[], stale: false },
        ],
    },
    F {
        id: "search",
        title: "검색",
        summary: "질문을 찾아 답까지",
        screens: &[
            S {
                id: "results",
                title: "검색 결과",
                note: Some("입력하는 대로 걸러요. 일치하는 말은 형광으로."),
                ticket: Some(160),
                states: &[("default", "기본", "default", None, Drawn::Both), ("empty", "빈 상태", "empty", Some("아무것도 찾지 못했을 때"), Drawn::Both), ("loading", "로딩", "loading", None, Drawn::Phone), ("long", "긴 글", "long", None, Drawn::Phone)],
                missing: &[],
                stale: false,
            },
            S { id: "question", title: "질문 상세", note: None, ticket: None, states: &[("default", "기본", "default", None, Drawn::Both)], missing: &["error", "permission"], stale: false },
        ],
    },
    F {
        id: "settings",
        title: "설정",
        summary: "프로필과 알림",
        screens: &[S { id: "profile", title: "프로필", note: None, ticket: None, states: &[("default", "기본", "default", None, Drawn::Both), ("offline", "오프라인", "offline", None, Drawn::Phone)], missing: &[], stale: false }],
    },
];

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn shots(drawn: Drawn) -> Vec<(&'static str, u32)> {
    match drawn {
        Drawn::No => vec![],
        Drawn::Phone => vec![("light", 390), ("dark", 390)],
        Drawn::Both => vec![("light", 390), ("dark", 390), ("light", 1280), ("dark", 1280)],
    }
}

fn picture_path(design: &Path, flow: &str, screen: &str, state: &str, mode: &str, width: u32) -> PathBuf {
    design.join("flows").join(flow).join(screen).join(format!("{state}.{mode}.{width}.png"))
}

/// The map, by the pictures written under `design`.
pub fn map(design: &Path) -> FlowMap {
    let taken = now().saturating_sub(2 * 86_400);
    let flows = FLOWS
        .iter()
        .map(|f| Flow {
            id: f.id.into(),
            title: f.title.into(),
            summary: Some(f.summary.into()),
            screens: f
                .screens
                .iter()
                .map(|s| Screen {
                    id: s.id.into(),
                    title: s.title.into(),
                    note: s.note.map(Into::into),
                    url: Some(format!("http://localhost:3000/{}", s.id)),
                    source: vec![format!("src/app/{}/page.tsx", s.id)],
                    ticket: s.ticket,
                    states: s
                        .states
                        .iter()
                        .map(|(id, label, kind, note, drawn)| ScreenState {
                            id: (*id).into(),
                            label: (*label).into(),
                            kind: (*kind).into(),
                            note: note.map(Into::into),
                            pictures: shots(*drawn)
                                .into_iter()
                                .map(|(mode, width)| Picture { path: picture_path(design, f.id, s.id, id, mode, width).to_string_lossy().into_owned(), mode: mode.into(), width, taken: if s.stale { taken - 12 * 86_400 } else { taken }, hash: None })
                                .collect(),
                        })
                        .collect(),
                    missing: s.missing.iter().map(|m| (*m).to_owned()).collect(),
                    stale: s.stale,
                })
                .collect(),
            videos: videos(design, f.id),
        })
        .collect();
    FlowMap { flows, problems: vec!["flows/search/results/loading.sepia.390.png: 모드는 light나 dark여야 해요".into()], unlisted: vec!["onboarding/login/eror.dark.390.png".into()] }
}

fn video_path(design: &Path, flow: &str) -> PathBuf {
    design.join("flows").join(flow).join("walkthrough.light.390.mp4")
}

fn videos(design: &Path, flow: &str) -> Vec<Walkthrough> {
    let mark = |screen: &str, at: u64| Mark { screen: screen.into(), at };
    match flow {
        "onboarding" => vec![
            Walkthrough {
                path: video_path(design, flow).to_string_lossy().into_owned(),
                mode: "light".into(),
                width: 390,
                taken: now().saturating_sub(3 * 3600),
                millis: 9_000,
                marks: vec![mark("login", 0), mark("signup", 3_000), mark("welcome", 6_000)],
                current: true,
            },
            Walkthrough {
                path: design.join("flows/onboarding/walkthrough-old.light.390.mp4").to_string_lossy().into_owned(),
                mode: "light".into(),
                width: 390,
                taken: now().saturating_sub(9 * 86_400),
                millis: 14_200,
                marks: vec![mark("login", 0), mark("signup", 5_100)],
                current: false,
            },
        ],
        // Made on the daemon's other Mac: not here.
        "checkout" => vec![Walkthrough {
            path: "/Volumes/studio/equipe/web/design/flows/checkout/walkthrough.dark.1280.mp4".into(),
            mode: "dark".into(),
            width: 1280,
            taken: now().saturating_sub(26 * 3600),
            millis: 41_000,
            marks: vec![mark("plans", 0), mark("pay", 12_500), mark("receipt", 33_000)],
            current: true,
        }],
        _ => vec![],
    }
}

/// The map on the web project's system — and on T-170's draft of it — as
/// the daemon will send it.
pub fn dress(systems: &mut [DesignSystem]) {
    for system in systems.iter_mut().filter(|s| s.project.as_deref() == Some("web") && s.flows.is_none()) {
        system.flows = Some(map(Path::new(&system.dir)));
    }
}

/// The screens T-170 shot again, its accent violet and its main button rounder.
const REFRESHED: [&str; 5] = ["login", "welcome", "plans", "pay", "profile"];

/// T-170's worktree: the map as the web project has it — copied, so the
/// same pictures but saved at another time — and the screens it refreshed
/// drawn again.
pub fn seed_draft(draft: &Path, accepted: &Path) {
    for (fi, f) in FLOWS.iter().enumerate() {
        for (si, s) in f.screens.iter().enumerate() {
            for (state, _, kind, _, drawn) in s.states {
                for (mode, width) in shots(*drawn) {
                    let path = picture_path(draft, f.id, s.id, state, mode, width);
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    // The refresh shows in the default states (and the welcome's first).
                    let redrawn = REFRESHED.contains(&s.id) && matches!(*kind, "default" | "first" | "error");
                    if redrawn {
                        paint(kind, mode == "dark", width, fi * 7 + si * 3, true).save(&path).unwrap();
                    } else {
                        let _ = std::fs::copy(picture_path(accepted, f.id, s.id, state, mode, width), &path);
                    }
                }
            }
        }
    }
    let _ = std::fs::copy(video_path(accepted, "onboarding"), video_path(draft, "onboarding"));
}

/// `FAKE_DESIGN=working`: T-378's worktree mid-way through the squad's
/// map — `flows.json` naming 온보딩 and 결제, read the daemon's way; 온보딩
/// shot, 결제 only its first screen so far.
pub fn seed_working(design: &Path) {
    let flows: Vec<serde_json::Value> = FLOWS
        .iter()
        .take(2)
        .map(|f| {
            let screens: Vec<serde_json::Value> = f
                .screens
                .iter()
                .map(|s| {
                    let states: Vec<serde_json::Value> = s.states.iter().map(|(id, label, kind, note, _)| serde_json::json!({ "id": id, "label": label, "kind": kind, "note": note })).collect();
                    serde_json::json!({ "id": s.id, "title": s.title, "note": s.note, "states": states })
                })
                .collect();
            serde_json::json!({ "id": f.id, "title": f.title, "summary": f.summary, "screens": screens })
        })
        .collect();
    std::fs::create_dir_all(design.join("flows")).unwrap();
    std::fs::write(design.join("flows/flows.json"), serde_json::to_string_pretty(&serde_json::json!({ "flows": flows })).unwrap()).unwrap();
    for (fi, f) in FLOWS.iter().take(2).enumerate() {
        let shot = if fi == 0 { f.screens.len() } else { 1 };
        for (si, s) in f.screens.iter().enumerate().take(shot) {
            for (state, _, kind, _, drawn) in s.states {
                for (mode, width) in shots(*drawn) {
                    let path = picture_path(design, f.id, s.id, state, mode, width);
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    draw(kind, mode == "dark", width, fi * 7 + si * 3).save(&path).unwrap();
                }
            }
        }
    }
}

/// Every picture the map names, for `read_image`.
pub fn pictures(systems: &[DesignSystem]) -> Vec<String> {
    systems.iter().filter_map(|s| s.flows.as_ref()).flat_map(|m| &m.flows).flat_map(|f| &f.screens).flat_map(|s| &s.states).flat_map(|s| &s.pictures).map(|p| p.path.clone()).collect()
}

/// Draw the pictures (and the walkthrough) under the web project's `design/`.
pub fn seed(design: &Path) {
    let mut frames: Vec<PathBuf> = Vec::new();
    for (fi, f) in FLOWS.iter().enumerate() {
        for (si, s) in f.screens.iter().enumerate() {
            for (state, _, kind, _, drawn) in s.states {
                for (mode, width) in shots(*drawn) {
                    let path = picture_path(design, f.id, s.id, state, mode, width);
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    draw(kind, mode == "dark", width, fi * 7 + si * 3).save(&path).unwrap();
                    // The walkthrough's frames: each onboarding screen as first met.
                    let first = if s.id == "welcome" { *kind == "first" } else { *kind == "default" };
                    if f.id == "onboarding" && first && (mode, width) == ("light", 390) {
                        frames.push(path);
                    }
                }
            }
        }
    }
    // The walkthrough: each screen three seconds, when `ffmpeg` is here.
    let dir = std::env::temp_dir().join(format!("equipe-fake-walk-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    for (n, frame) in frames.iter().take(3).enumerate() {
        let _ = std::fs::copy(frame, dir.join(format!("f{n}.png")));
    }
    let out = video_path(design, "onboarding");
    let made = std::process::Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-framerate", "1/3", "-i"])
        .arg(dir.join("f%d.png"))
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-r", "10"])
        .arg(&out)
        .status()
        .is_ok_and(|s| s.success());
    if !made {
        let _ = std::fs::remove_file(&out);
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// A placeholder of a screen in a state: a bar, a title, blocks — the
/// state's own mark (an error's red line, an empty state's ring, loading's
/// grey bars, a long text's many lines).
fn draw(kind: &str, dark: bool, width: u32, seed: usize) -> image::RgbaImage {
    paint(kind, dark, width, seed, false)
}

/// The same, `refreshed` as T-170 has it: violet, the main button rounder and narrower.
fn paint(kind: &str, dark: bool, width: u32, seed: usize, refreshed: bool) -> image::RgbaImage {
    let (w, h) = if width < 600 { (390u32, 780u32) } else { (1280u32, 800u32) };
    let rgb = |c: u32| image::Rgba([(c >> 16) as u8, (c >> 8) as u8, c as u8, 255]);
    let (bg, surface, ink, soft) = if dark { (0x0A0A0C, 0x16171A, 0xEFEDE7, 0x3A3B40) } else { (0xF6F5F1, 0xFFFFFF, 0x15161A, 0xD9D6CF) };
    let accent = match (refreshed, dark) {
        (false, true) => 0x8FB0FF,
        (false, false) => 0x1D4ED8,
        (true, true) => 0xC4B5FD,
        (true, false) => 0x6D28D9,
    };
    let mut img = image::RgbaImage::from_pixel(w, h, rgb(bg));
    let rect = |img: &mut image::RgbaImage, x: u32, y: u32, rw: u32, rh: u32, c: u32| {
        for yy in y..(y + rh).min(h) {
            for xx in x..(x + rw).min(w) {
                img.put_pixel(xx, yy, rgb(c));
            }
        }
    };
    let m = if width < 600 { 24 } else { 120 };
    let cw = w - 2 * m;
    rect(&mut img, 0, 0, w, 56, surface);
    rect(&mut img, m, 20, 72, 16, ink);
    rect(&mut img, m, 96, cw * (5 + (seed % 4) as u32) / 10, 28, ink);
    match kind {
        "loading" => {
            for i in 0..5 {
                rect(&mut img, m, 160 + i * 44, cw - (i % 3) * 40, 24, soft);
            }
        }
        "empty" => {
            let (cx, cy, r) = (w / 2, h / 2 - 40, 44i64);
            for yy in 0..h {
                for xx in 0..w {
                    let d = ((xx as i64 - cx as i64).pow(2) + (yy as i64 - cy as i64).pow(2)) as f64;
                    if (d.sqrt() - r as f64).abs() < 3. {
                        img.put_pixel(xx, yy, rgb(soft));
                    }
                }
            }
            rect(&mut img, w / 2 - 80, h / 2 + 30, 160, 14, soft);
            rect(&mut img, w / 2 - 60, h / 2 + 70, 120, 36, accent);
        }
        "long" => {
            for i in 0..14 {
                rect(&mut img, m, 150 + i * 30, cw - (i * 37 % 90), 12, soft);
            }
        }
        _ => {
            for i in 0..3 {
                rect(&mut img, m, 160 + i * 76, cw, 56, surface);
                rect(&mut img, m + 16, 180 + i * 76, cw / 2, 14, soft);
            }
            if kind == "error" {
                rect(&mut img, m, 400, cw, 4, if dark { 0xEE7B74 } else { 0xAD2E27 });
                rect(&mut img, m, 414, cw * 2 / 3, 12, if dark { 0xEE7B74 } else { 0xAD2E27 });
            }
            if kind == "permission" || kind == "offline" {
                rect(&mut img, 0, 56, w, 40, if dark { 0x5C4A12 } else { 0xFFE58A });
            }
            let fill = if kind == "success" { 0x1F6B33 } else { accent };
            if refreshed {
                // A pill, centred, and the header's mark in the new colour.
                let bw = cw * 3 / 5;
                rect(&mut img, (w - bw) / 2, h - 124, bw, 56, fill);
                rect(&mut img, m, 20, 72, 16, accent);
            } else {
                rect(&mut img, m, h - 120, cw, 48, fill);
            }
        }
    }
    img
}
