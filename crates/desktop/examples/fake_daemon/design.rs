//! `FAKE_DESIGN`: the squad's design system, as a designer keeps it in
//! `design/` (`docs/DESIGN-SYSTEM.md`) — modelled on T-144's brand guideline
//! (equipe's own: warm neutrals, one signal blue, Inter and Pretendard), its
//! folders written under `$EQUIPE_HOME/design-system` and read the
//! daemon's way (`equipe_core::design`), pictures and all.
//!
//! - `1`: the squad's, and the web project's own (the site's older palette).
//! - `draft`: the squad's as the desktop has it now (steel blue), and T-144
//!   in review with its draft (signal blue) — the "검토 중" pill.
//! - `empty`: none yet.
//!
//! `FAKE_DESIGN_CHANGE_AFTER=<ms>`: the designer saves new tokens that long
//! after the first ask — an accent moved, a colour added — for "바뀐 것".
//! The folders are watched as the daemon watches them: edit them by hand and
//! the screen follows.

use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_core::design::folder::{self, Place};
use equipe_core::i18n::Lang;
use equipe_core::protocol::design::DraftOf;
use equipe_core::protocol::rpc::{ImageRead, ReadImage};
use equipe_core::protocol::{Command, Event, Ticket};
use tokio::sync::broadcast;

pub fn mode() -> Option<String> {
    std::env::var("FAKE_DESIGN").ok().filter(|v| !v.is_empty() && v != "0")
}

/// Under the fake daemon's own home, so two of them never share it.
fn base() -> PathBuf {
    equipe_providers::credentials::equipe_home().join("design-system")
}

/// Where each fake design system lives.
fn places() -> Vec<Place> {
    let mut out = vec![Place { project: None, draft: None, dir: base().join("squad/design") }];
    match mode().as_deref() {
        Some("draft") => out.push(Place { project: None, draft: Some(DraftOf { ticket: 144, title: "브랜드 가이드라인".into() }), dir: base().join("t144/design") }),
        Some("empty") | None => {}
        Some(_) => out.push(Place { project: Some("web".into()), draft: None, dir: base().join("web/design") }),
    }
    out
}

/// T-144, in review, when its draft is shown.
pub fn ticket() -> Option<Ticket> {
    (mode().as_deref() == Some("draft")).then(|| {
        let mut t = super::ticket(
            144,
            "review",
            "app",
            "designer",
            "브랜드 가이드라인",
            "- [ ] 로고·색·서체·목소리 정리\n- [ ] 라이트·다크 대비 AA 이상\n- [ ] `design/`에 토큰과 가이드라인",
            &[],
            None,
            Some("Signal blue 안으로 `design/`을 정리했습니다. 색 토큰 라이트·다크, 서체, 간격, 모션, 로고 세 가지."),
        );
        t.kind = equipe_core::protocol::TicketKind::Task;
        t.project = None;
        t
    })
}

/// Write the folders. Called once the fake repositories are made.
pub fn seed() {
    let Some(mode) = mode() else { return };
    let _ = std::fs::remove_dir_all(base());
    if mode == "empty" {
        return;
    }
    let squad = base().join("squad/design");
    if mode == "draft" {
        write_system(&squad, &tokens(&STEEL), GUIDELINES_STEEL);
        write_system(&base().join("t144/design"), &tokens(&SIGNAL), GUIDELINES);
    } else {
        write_system(&squad, &tokens(&SIGNAL), GUIDELINES);
        write_system(&base().join("web/design"), &tokens(&SITE), GUIDELINES_WEB);
    }
}

/// A palette: light and dark, by role.
struct Palette {
    bg: (&'static str, &'static str),
    surface: (&'static str, &'static str),
    sunken: (&'static str, &'static str),
    text: (&'static str, &'static str),
    muted: (&'static str, &'static str),
    accent: (&'static str, &'static str),
    on_accent: (&'static str, &'static str),
    dot: (&'static str, &'static str),
}

/// T-144's recommendation A: warm neutrals and one blue.
const SIGNAL: Palette = Palette {
    bg: ("#F6F5F1", "#0A0A0C"),
    surface: ("#FFFFFF", "#16171A"),
    sunken: ("#EAE7E0", "#111114"),
    text: ("#15161A", "#EFEDE7"),
    muted: ("#4F5159", "#A3A5AB"),
    accent: ("#1D4ED8", "#8FB0FF"),
    on_accent: ("#FFFFFF", "#0B1226"),
    dot: ("#3F6FF5", "#3F6FF5"),
};

/// B, as the desktop has it now: steel blue, a grey dot.
const STEEL: Palette = Palette {
    bg: ("#F6F5F1", "#0A0A0C"),
    surface: ("#FFFFFF", "#141417"),
    sunken: ("#ECEBE6", "#111114"),
    text: ("#15161A", "#EFEDE7"),
    muted: ("#5E6068", "#8A8C93"),
    accent: ("#2E5C8A", "#B4CDE6"),
    on_accent: ("#F6F5F1", "#0A0A0C"),
    dot: ("#7F8591", "#8A909B"),
};

/// The site's own, as it was before T-144.
const SITE: Palette = Palette {
    bg: ("#FAF8F4", "#121211"),
    surface: ("#F0EDE6", "#1B1B19"),
    sunken: ("#EAE6DD", "#262623"),
    text: ("#1A1A17", "#F2EFE8"),
    muted: ("#57544C", "#A9A59B"),
    accent: ("#1D4ED8", "#8FB0FF"),
    on_accent: ("#FFFFFF", "#0B1226"),
    dot: ("#1D4ED8", "#8FB0FF"),
};

/// `tokens.json` for a palette: colours as light and dark groups (as T-144
/// wrote them), status colours with `$extensions.mode`, aliases, type, space,
/// radius, shadow, motion.
fn tokens(p: &Palette) -> String {
    let side = |light: bool| {
        let v = |pair: (&str, &str)| if light { pair.0.to_owned() } else { pair.1.to_owned() };
        serde_json::json!({
            "bg": { "$value": v(p.bg), "$description": "페이지·앱 바탕" },
            "surface": { "$value": v(p.surface), "$description": "카드·시트" },
            "sunken": { "$value": v(p.sunken), "$description": "코드·눌린 면" },
            "text": { "$value": v(p.text), "$description": "본문·제목" },
            "muted": { "$value": v(p.muted), "$description": "보조 글자" },
            "accent": { "$value": v(p.accent), "$description": "링크·선택·포커스" },
            "onAccent": { "$value": v(p.on_accent), "$description": "채운 accent 위 글자" },
            "focus": { "$value": if light { "{color.light.accent}" } else { "{color.dark.accent}" } },
            "line": { "$value": if light { "rgba(10,10,12,.14)" } else { "rgba(233,230,223,.14)" }, "$description": "헤어라인" }
        })
    };
    let tokens = serde_json::json!({
        "$description": "equipe brand tokens — from T-144 (docs/DESIGN-SYSTEM.md)",
        "color": {
            "$type": "color",
            "light": side(true),
            "dark": side(false),
            "status": {
                "ok": { "$value": "#1F6B33", "$extensions": { "mode": { "dark": "#7FCB8E" } }, "$description": "완료" },
                "work": { "$value": "#825600", "$extensions": { "mode": { "dark": "#E2B865" } }, "$description": "진행 중" },
                "wait": { "$value": "#A04610", "$extensions": { "mode": { "dark": "#EC9A62" } }, "$description": "내 차례·허락 대기" },
                "err": { "$value": "#AD2E27", "$extensions": { "mode": { "dark": "#EE7B74" } }, "$description": "오류" }
            },
            "link": { "$value": "{color.accent}" }
        },
        "brand": {
            "$type": "color",
            "dot": { "$value": p.dot.0, "$extensions": { "mode": { "dark": p.dot.1 } }, "$description": "브랜드 점 — 로고 전용 그래픽" },
            "tile": { "$value": "#0A0A0C", "$description": "마크 타일 (void)" },
            "mark-fg": { "$value": "#EFEDE7", "$description": "마크 글자 (bone)" }
        },
        "font": {
            "$type": "fontFamily",
            "sans": { "$value": ["Inter", "Pretendard Variable", "Apple SD Gothic Neo", "system-ui", "sans-serif"] },
            "mono": { "$value": ["JetBrains Mono", "ui-monospace", "Menlo", "monospace"] },
            "weight": {
                "$type": "fontWeight",
                "regular": { "$value": 400 },
                "semibold": { "$value": "semi-bold" },
                "bold": { "$value": 700 }
            }
        },
        "type": {
            "$type": "typography",
            "display": { "$value": { "fontFamily": "{font.sans}", "fontSize": "56px", "fontWeight": "{font.weight.bold}", "lineHeight": 1.05, "letterSpacing": "-0.02em" }, "$description": "사이트 H1 (모바일 36)" },
            "title": { "$value": { "fontFamily": "{font.sans}", "fontSize": "32px", "fontWeight": "{font.weight.bold}", "lineHeight": 1.2 } },
            "heading": { "$value": { "fontFamily": "{font.sans}", "fontSize": "18px", "fontWeight": "{font.weight.semibold}", "lineHeight": "26px" } },
            "body": { "$value": { "fontFamily": "{font.sans}", "fontSize": "15px", "fontWeight": "{font.weight.regular}", "lineHeight": "25px" }, "$description": "본문 15/25 — 한국어 1.65 행간" },
            "small": { "$value": { "fontFamily": "{font.sans}", "fontSize": "13px", "fontWeight": "{font.weight.regular}", "lineHeight": "20px" } },
            "code": { "$value": { "fontFamily": "{font.mono}", "fontSize": "12.5px", "fontWeight": 400, "lineHeight": "19px" } }
        },
        "space": {
            "$type": "dimension",
            "1": { "$value": "4px" }, "2": { "$value": "8px" }, "3": { "$value": "12px" }, "4": { "$value": "16px" },
            "6": { "$value": "24px" }, "8": { "$value": "32px" }, "12": { "$value": "48px" }, "16": { "$value": "64px" }
        },
        "radius": {
            "$type": "dimension",
            "sm": { "$value": "6px", "$description": "칩·작은 버튼" },
            "md": { "$value": "8px", "$description": "컨트롤·메뉴·줄" },
            "lg": { "$value": "12px", "$description": "카드" },
            "xl": { "$value": "16px", "$description": "입력창·대화 상자" },
            "tile": { "$value": "14px", "$description": "마크 타일 (64 기준 22%)" }
        },
        "shadow": {
            "$type": "shadow",
            "card": { "$value": { "color": "#0000001A", "offsetX": "0px", "offsetY": "1px", "blur": "3px", "spread": "0px" } },
            "float": { "$value": [{ "color": "#00000038", "offsetX": "0px", "offsetY": "8px", "blur": "24px", "spread": "-4px" }, { "color": "#0000001A", "offsetX": "0px", "offsetY": "2px", "blur": "6px", "spread": "0px" }], "$description": "메뉴·팝오버" },
            "dialog": { "$value": { "color": "#00000047", "offsetX": "0px", "offsetY": "24px", "blur": "48px", "spread": "-12px" } }
        },
        "motion": {
            "duration": {
                "$type": "duration",
                "fast": { "$value": "120ms", "$description": "호버·툴팁·누름" },
                "base": { "$value": "160ms", "$description": "팝오버·메뉴" },
                "slow": { "$value": "200ms", "$description": "대화 상자·카드 도착" },
                "slide": { "$value": "180ms", "$description": "사이드바" }
            },
            "ease": {
                "$type": "cubicBezier",
                "out": { "$value": [0.33, 1, 0.68, 1], "$description": "하나뿐인 곡선 — 빨리 시작, 부드럽게 도착" },
                "slide": { "$value": [0.22, 1, 0.36, 1] },
                "in-out": { "$value": "ease-in-out" }
            },
            "pop": { "$type": "transition", "$value": { "duration": "{motion.duration.base}", "delay": "0ms", "timingFunction": "{motion.ease.out}" } }
        }
    });
    serde_json::to_string_pretty(&tokens).unwrap()
}

fn write_system(dir: &Path, tokens: &str, guidelines: &str) {
    for sub in ["assets/logo", "assets/icons", "assets/illustrations", "components"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
    }
    std::fs::write(dir.join("tokens.json"), tokens).unwrap();
    std::fs::write(dir.join("guidelines.md"), guidelines).unwrap();
    let dot = if tokens.contains("#7F8591") { "#7F8591" } else { "#3F6FF5" };
    let files: [(&str, String); 11] = [
        ("assets/logo/mark.svg", mark("#0A0A0C", "#EFEDE7", dot)),
        ("assets/logo/mark-mono-on-dark.svg", mark("none", "#EFEDE7", "#EFEDE7")),
        ("assets/logo/mark-mono-on-light.svg", mark("none", "#15161A", "#15161A")),
        ("assets/icons/inbox.svg", icon(r#"<path d="M22 12h-6l-2 3h-4l-2-3H2"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/>"#)),
        ("assets/icons/check.svg", icon(r#"<path d="M20 6 9 17l-5-5"/>"#)),
        ("assets/icons/bell.svg", icon(r#"<path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/>"#)),
        ("assets/illustrations/four-dots.svg", four_dots(dot)),
        ("components/button.svg", button(dot)),
        ("components/card.svg", card()),
        ("components/input.svg", input()),
        ("assets/illustrations/README.txt", "not a picture: left out of the grid".into()),
    ];
    for (name, body) in files {
        std::fs::write(dir.join(name), body).unwrap();
    }
    // A raster one too: the social card, a PNG with the four dots on void.
    social_card(dot).save(dir.join("assets/illustrations/social-card.png")).unwrap();
}

/// The mark: a tile (or none), a lowercase `e` drawn as a stroke, the dot.
fn mark(tile: &str, ink: &str, dot: &str) -> String {
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024"><rect width="1024" height="1024" rx="225" fill="{tile}"/><path d="M300 540 H650 A182 182 0 1 0 600 668" fill="none" stroke="{ink}" stroke-width="86"/><circle cx="772" cy="690" r="64" fill="{dot}"/></svg>"##
    )
}

fn icon(body: &str) -> String {
    format!(r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="96" height="96" fill="none" stroke="#4F5159" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{body}</svg>"##)
}

fn four_dots(dot: &str) -> String {
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 200"><circle cx="64" cy="64" r="34" fill="{dot}"/><circle cx="136" cy="64" r="32" fill="none" stroke="#A3A5AB" stroke-width="4"/><circle cx="64" cy="136" r="32" fill="none" stroke="#A3A5AB" stroke-width="4"/><circle cx="136" cy="136" r="32" fill="none" stroke="#A3A5AB" stroke-width="4"/></svg>"##
    )
}

fn button(dot: &str) -> String {
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 320 120"><rect x="20" y="36" width="132" height="44" rx="8" fill="#15161A"/><rect x="48" y="54" width="76" height="8" rx="4" fill="#F6F5F1"/><rect x="168" y="36" width="132" height="44" rx="8" fill="none" stroke="#0A0A0C24" stroke-width="2"/><rect x="196" y="54" width="62" height="8" rx="4" fill="#15161A"/><circle cx="276" cy="58" r="5" fill="{dot}"/></svg>"##
    )
}

fn card() -> String {
    r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 320 200"><rect x="24" y="24" width="272" height="152" rx="12" fill="#FFFFFF" stroke="#0A0A0C24" stroke-width="2"/><rect x="44" y="48" width="120" height="12" rx="6" fill="#15161A"/><rect x="44" y="76" width="220" height="8" rx="4" fill="#4F515966"/><rect x="44" y="94" width="190" height="8" rx="4" fill="#4F515966"/><rect x="44" y="132" width="72" height="24" rx="6" fill="#1D4ED8"/></svg>"##.into()
}

fn input() -> String {
    r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 320 120"><rect x="20" y="38" width="280" height="44" rx="16" fill="#FFFFFF" stroke="#1D4ED8" stroke-width="2"/><rect x="40" y="56" width="2" height="10" fill="#15161A"/><rect x="52" y="57" width="120" height="8" rx="4" fill="#4F515955"/><circle cx="276" cy="60" r="12" fill="#15161A"/></svg>"##.into()
}

/// 1200×630: void, the four dots, one of them the brand's.
fn social_card(dot: &str) -> image::RgbaImage {
    let blue = u32::from_str_radix(dot.trim_start_matches('#'), 16).unwrap_or(0x3F6FF5);
    let (bw, bh) = (1200u32, 630u32);
    image::RgbaImage::from_fn(bw, bh, |x, y| {
        let d = |cx: f32, cy: f32| ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
        let filled = d(520., 250.) < 70.;
        let ring = [(680., 250.), (520., 410.), (680., 410.)].iter().any(|(cx, cy)| (d(*cx, *cy) - 66.).abs() < 4.);
        if filled {
            image::Rgba([(blue >> 16) as u8, (blue >> 8) as u8, blue as u8, 255])
        } else if ring {
            image::Rgba([0xA3, 0xA5, 0xAB, 255])
        } else {
            image::Rgba([0x0A, 0x0A, 0x0C, 255])
        }
    })
}

/// The design systems as the daemon would send them now.
fn event() -> Event {
    let (systems, folder) = folder::catalog_of(&places(), Lang::Ko);
    Event::Design { squad: "demo".into(), systems, folder }
}

/// `design`: answered at once, and the folders watched from then on.
pub fn on_command(command: &Command, events: &broadcast::Sender<Event>) -> bool {
    let Command::Design { .. } = command else { return false };
    let _ = events.send(event());
    static WATCHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !WATCHING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        let events = events.clone();
        tokio::spawn(async move {
            let mut last = folder::stamp(&places());
            loop {
                tokio::time::sleep(Duration::from_millis(1500)).await;
                let now = folder::stamp(&places());
                if now != last {
                    last = now;
                    let _ = events.send(event());
                }
            }
        });
        if let Some(after) = std::env::var("FAKE_DESIGN_CHANGE_AFTER").ok().and_then(|v| v.parse::<u64>().ok()) {
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(after)).await;
                let dir = base().join("squad/design");
                let Ok(text) = std::fs::read_to_string(dir.join("tokens.json")) else { return };
                // The designer moves the accent a step and adds a highlight.
                let moved = text.replacen("\"#1D4ED8\"", "\"#2251E0\"", 1).replacen("\"#2E5C8A\"", "\"#2B5F94\"", 1).replacen(
                    "\"link\": {",
                    "\"highlight\": { \"$value\": \"#FFE58A\", \"$extensions\": { \"mode\": { \"dark\": \"#5C4A12\" } }, \"$description\": \"검색 일치\" },\n      \"link\": {",
                    1,
                );
                let _ = std::fs::write(dir.join("tokens.json"), moved);
            });
        }
    }
    true
}

/// `read_image` for a design system's picture, as the daemon checks it:
/// only one an asset names, inside its folder.
pub fn read_image(ask: &ReadImage) -> Result<ImageRead, String> {
    let places = places();
    let (systems, _) = folder::catalog_of(&places, Lang::En);
    if !systems.iter().any(|s| s.assets.iter().any(|a| a.path == ask.path)) {
        return Err(format!("{} isn't a picture {} shows", ask.path, ask.squad));
    }
    let roots: Vec<PathBuf> = places.into_iter().map(|p| p.dir).collect();
    equipe_core::visuals::read(Path::new(&ask.path), &roots, ask.max)
}

const GUIDELINES: &str = r#"# equipe 브랜드 가이드라인 v1

상태: **확정 v1** — 사람이 네 가지를 모두 추천안으로 골랐다(D1 ① · D2 A · D3 1 · D4 해요체). 토큰은 `tokens.json`, 그림은 `assets/`.

## 1. 로고

### 1.1 구성
- **마크 `e.`**: void `#0A0A0C` 타일 + bone `#EFEDE7` 소문자 `e` + 브랜드 점. 앱 아이콘·파비콘·아바타. 모서리 반지름 = 한 변의 22%.
- **점의 뜻**: 사이트 4점의 "꽉 찬 점 = 이끄는 사람"을 점 하나로 줄인 것. 4점은 본문 일러스트로 남긴다.

### 1.2 사용 규칙
- **여백**: 마크 한 변의 ¼ — 안쪽엔 아무것도 들어오지 않는다.
- **최소 크기**: 마크 16px. 16px에서도 점이 2px 이상 남는다.
- **배경**: 밝은 바탕에는 글자 `#15161A`, 어두운 바탕에는 bone `#EFEDE7`. 점은 양쪽 모두 `#3F6FF5`.
- **하지 말 것**: 점·글자 색 바꾸기, 늘이기·기울이기, 그림자·그라데이션, 사진 위에 얹기.

## 2. 색

### 2.1 원칙
1. **색은 둘뿐: 브랜드 파랑(단 하나)과 의미색.** 나머지는 따뜻한 중립(bone·void).
2. 라이트·다크 모두 같은 **역할 이름**을 쓴다. 값만 모드별로 다르다.
3. 글자 조합은 WCAG 2.x AA(4.5:1), 그래픽은 3:1 이상. 색만으로 상태를 전달하지 않는다.

### 2.2 역할 토큰

| 역할 | 라이트 | 다크 | 쓰임 |
|---|---|---|---|
| `bg` | `#F6F5F1` | `#0A0A0C` | 페이지·앱 바탕 |
| `surface` | `#FFFFFF` | `#16171A` | 카드·시트 |
| `text` | `#15161A` | `#EFEDE7` | 본문·제목 |
| `muted` | `#4F5159` | `#A3A5AB` | 보조 글자 |
| `accent` | `#1D4ED8` | `#8FB0FF` | 링크·선택·포커스 |
| `dot` | `#3F6FF5` | `#3F6FF5` | 브랜드 점(로고 전용) |

> 점(`dot`)의 "배경 대비 3:1"은 그래픽 기준이다. 점을 **글자 색**으로 쓰지 않는다(라이트 bg 위 4.01:1).

### 2.3 의미색
- amber = 일하는 중, orange = 사람을 기다림, red = 오류, green = 완료. 프로젝트 색으로 쓰지 않는다.

## 3. 서체

### 3.1 스택
- **웹**: Inter + Pretendard(ko) + Noto Sans(ja·zh). **앱**: 플랫폼 시스템 글꼴.
- **모노**: JetBrains Mono, 폴백 `ui-monospace, Menlo`.
- 굵기: 제목 700(자간 −0.02em, CJK는 0), 본문 400, 버튼·라벨 600.

### 3.2 한국어
- `word-break: keep-all`, 본문 행간 1.65. 12px 밑으로 쓰지 않는다.

## 4. 간격과 모서리
- 4 · 8 · 12 · 16 · 24 · 32 · 48 · 64. 그 사이 값은 쓰지 않는다.
- 반지름: 칩 6, 컨트롤 8, 카드 12, 입력창·대화 상자 16.

## 5. 모션
- 세 길이(120 · 160 · 200ms)와 곡선 하나(ease-out). 사이드바만 180ms quint.
- "동작 줄이기"를 켜면 끝 상태를 바로 보여 준다.

## 6. 목소리
짧게·능동·구체. 사람이 할 일을 먼저. 과장어·느낌표·이모지·ALL CAPS 장식 금지. 한국어는 **해요체**.
"#;

const GUIDELINES_STEEL: &str = r#"# equipe 디자인 시스템 (현행)

데스크톱 앱이 지금 쓰는 값이다. T-144의 브랜드 가이드라인이 받아들여지면 이 폴더가 그 안으로 바뀐다.

## 색
- 강조색은 강철 파랑 `#2E5C8A`(다크 `#B4CDE6`). 점은 크롬 회색.
- 제품 UI의 채운 주 버튼은 `text` 색 면.

## 서체
- 앱은 시스템 글꼴(Helvetica Neue), 모노는 Menlo.
"#;

const GUIDELINES_WEB: &str = r#"# 사이트 디자인 토큰

equipe.im 사이트가 쓰는 값. 스쿼드 디자인 시스템과 이름은 같고 값만 다르다.

## 색
- 바탕은 조금 더 따뜻한 `#FAF8F4`. 링크·버튼은 `#1D4ED8`.

## 서체
- Inter(자체 호스팅), 한국어 Pretendard. H1 56/1.05, 본문 18/1.6.
"#;
