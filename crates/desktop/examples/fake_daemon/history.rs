//! `FAKE_HISTORY=<n>`: the PM's history as a long day leaves it, `n` lines
//! of it — for measuring how the app holds up (see `docs/PERF-2026-10.md`).
//! Turns over the three projects and the whole squad: the person's message
//! (now and then with screenshots), a group of calls with their output and
//! changes, a long Korean reply in Markdown (paragraphs, lists, bold, ticket
//! links, a table or a code block every few), and news between turns.
//! The same `n` always gives the same history. `FAKE_LONG_REPLY=1` makes the
//! PM's streamed answer a long one of these too.

use equipe_core::protocol::{Entry, ToolOutcome};

const SITE: &str = "/tmp/fake/site";

const SENTENCES: [&str; 12] = [
    "지금 보드에 남은 일은 우선순위대로 정리해 두었고, 급한 것부터 멤버들에게 넘기고 있습니다.",
    "frontend는 상품 상세 페이지의 이미지 갤러리를 마무리하는 중이고, 오늘 오후에 검토를 요청할 예정이에요.",
    "backend가 주문 API의 페이지네이션을 바꾸면서 기존 클라이언트가 깨지지 않도록 예전 형식도 함께 받게 했습니다.",
    "테스트는 전부 통과했지만 결제 흐름의 끝부분은 실제 카드로 한 번 더 확인해 보는 게 좋겠습니다.",
    "디자이너가 보낸 온보딩 시안은 세 화면 모두 받았고, 두 번째 화면의 문구만 조금 다듬으면 될 것 같아요.",
    "배포는 되돌리기 어려운 일이라 항상 허락을 받고 진행하니, 준비가 되면 알려 드리겠습니다.",
    "어제 말씀하신 검색 개선은 인덱스를 다시 만드는 작업이 필요해서 시간이 조금 더 걸립니다.",
    "로그를 보니 새벽에 요청이 몰리는 시간대가 있어서, 그때만 캐시를 길게 두도록 바꾸는 게 어떨까 합니다.",
    "앱스토어 심사 연락처가 아직 비어 있어서 제출이 멈춰 있는데, 이건 직접 입력해 주셔야 해요.",
    "Internationalization 작업은 문자열을 모두 한곳으로 모은 뒤에 번역을 붙이는 순서로 진행하고 있습니다.",
    "성능 측정 결과는 첫 화면이 뜨는 데 평균 1.2초, 스크롤은 대부분 프레임이 8ms 안에 그려졌습니다.",
    "남은 질문이 있으면 이 대화에 바로 남겨 주세요. 다음 일을 고를 때 참고하겠습니다.",
];

const ASKS: [&str; 8] = [
    "오늘 할 일 정리해 줘",
    "상품 상세 갤러리는 어디까지 됐어?",
    "주문 API 바뀐 거 클라이언트에 영향 없어?",
    "온보딩 두 번째 화면도 같은 톤으로 맞춰 줘",
    "어제 배포 이후로 에러 늘었는지 봐 줘",
    "이번 주 우선순위 다시 정리해 줘",
    "이 화면 이상한데 확인 좀 해 줘",
    "검색 느린 거 원인 찾아 줘",
];

const CODE: &str = "```rust\n/// 주문 목록을 한 쪽씩 돌려줍니다.\npub async fn orders(db: &Db, after: Option<OrderId>, limit: u32) -> Result<Page<Order>> {\n    let limit = limit.clamp(1, 100);\n    let rows = db.query(\"select * from orders where id > $1 order by id limit $2\", (after.unwrap_or_default(), limit + 1)).await?;\n    let more = rows.len() > limit as usize;\n    Ok(Page { items: rows.into_iter().take(limit as usize).collect(), more })\n}\n```";

const TSX: &str = "```tsx\nexport function Gallery({ images }: { images: string[] }) {\n  const [at, setAt] = useState(0);\n  return (\n    <div className=\"flex gap-2 overflow-x-auto\">\n      {images.map((src, i) => <img key={src} src={src} onClick={() => setAt(i)} className={i === at ? 'ring-2' : ''} />)}\n    </div>\n  );\n}\n```";

const TABLE: &str = "| 티켓 | 프로젝트 | 담당 | 상태 |\n|---|---|---|---|\n| T-47 | web | frontend | 검토 대기 |\n| T-60 | app | designer | 진행 중 |\n| T-70 | api | backend | 진행 중 |\n| T-117 | app | frontend | 여러분 차례 |";

/// A small deterministic generator, so a history is the same each run.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// A long reply: three to six paragraphs, a list, and now and then a table or code.
fn reply(rng: &mut Rng, turn: u64) -> String {
    let mut out = String::new();
    let ticket = 40 + turn % 90;
    out.push_str(&format!("네, **T-{ticket}** 기준으로 정리했습니다.\n\n"));
    for _ in 0..3 + rng.below(4) {
        for _ in 0..2 + rng.below(3) {
            out.push_str(SENTENCES[rng.below(SENTENCES.len() as u64) as usize]);
            out.push(' ');
        }
        out.push_str("\n\n");
    }
    if rng.below(3) == 0 {
        out.push_str("## 진행 상황\n\n");
    }
    for k in 0..2 + rng.below(4) {
        out.push_str(&format!("- **T-{}** {}\n", ticket + k, SENTENCES[rng.below(SENTENCES.len() as u64) as usize]));
    }
    out.push('\n');
    match rng.below(6) {
        0 => out.push_str(&format!("{TABLE}\n\n")),
        1 => out.push_str(&format!("{CODE}\n\n")),
        2 => out.push_str(&format!("{TSX}\n\n")),
        _ => {}
    }
    out.push_str("끝나면 다시 알려 드리겠습니다.");
    out
}

fn picture(n: u64) -> String {
    let path = format!("{SITE}/history-{n}.png");
    if !std::path::Path::new(&path).exists() {
        let (w, h) = (1600u32, 1000u32);
        let mut img = image::RgbaImage::new(w, h);
        for (x, y, px) in img.enumerate_pixels_mut() {
            let t = x as f32 / w as f32;
            let s = y as f32 / h as f32;
            *px = image::Rgba([(60. + 150. * t) as u8, (90. + 100. * s) as u8, (40 * n % 255) as u8, 255]);
        }
        let _ = std::fs::create_dir_all(SITE);
        let _ = img.save(&path);
    }
    path
}

/// The PM's history, `n` lines of it.
pub fn history(n: usize) -> Vec<Entry> {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut out = Vec::with_capacity(n + 16);
    let projects = [Some("app"), Some("web"), Some("api"), None];
    let mut turn = 0u64;
    while out.len() < n {
        if turn.is_multiple_of(6) {
            let project = projects[rng.below(4) as usize];
            out.push(Entry::Thread { project: project.map(str::to_owned) });
        }
        let mut said = ASKS[rng.below(ASKS.len() as u64) as usize].to_owned();
        if turn % 12 == 5 {
            for k in 0..1 + rng.below(3) {
                said.push_str(&format!(" [image: {}]", picture((turn + k) % 6)));
            }
        }
        out.push(Entry::User { text: said });
        for k in 0..rng.below(9) {
            let file = format!("{SITE}/src/{}.rs", ["orders", "gallery", "search", "onboarding", "limit"][((turn + k) % 5) as usize]);
            let entry = match rng.below(4) {
                0 => Entry::Tool { tool: "Read".into(), target: Some(file), outcome: Some(Box::new(ToolOutcome { ok: true, ms: Some(40), ..Default::default() })) },
                1 => {
                    let output: String = (0..5 + rng.below(25)).map(|l| format!("test orders::page_{l} ... ok\n")).collect();
                    Entry::Tool { tool: "Bash".into(), target: Some("cargo test -p api".into()), outcome: Some(Box::new(ToolOutcome { ok: true, exit: Some(0), ms: Some(8_400), output, ..Default::default() })) }
                }
                2 => {
                    let diff = format!("diff --git a/{file} b/{file}\n--- a/{file}\n+++ b/{file}\n@@ -10,3 +10,4 @@\n     let limit = limit.clamp(1, 100);\n-    let rows = db.query(SQL, (after, limit)).await?;\n+    let rows = db.query(SQL, (after, limit + 1)).await?;\n+    let more = rows.len() > limit as usize;\n");
                    Entry::Tool { tool: "Edit".into(), target: Some(file), outcome: Some(Box::new(ToolOutcome { ok: true, ms: Some(12), diff: Some(diff), ..Default::default() })) }
                }
                _ => Entry::Tool { tool: "Grep".into(), target: Some("fn orders".into()), outcome: Some(Box::new(ToolOutcome { ok: true, ms: Some(90), ..Default::default() })) },
            };
            out.push(entry);
        }
        out.push(Entry::Reply { text: reply(&mut rng, turn) });
        if rng.below(3) == 0 {
            for k in 0..1 + rng.below(3) {
                let t = 40 + (turn + k) % 90;
                out.push(Entry::Note { text: format!("frontend handed in T-{t} for your review · 상품 상세 페이지 이미지 갤러리.") });
            }
        }
        turn += 1;
    }
    out.truncate(n);
    out
}

/// The long reply `FAKE_LONG_REPLY=1` streams.
pub fn long_reply() -> String {
    let mut rng = Rng(0xDEAD_BEEF_1234_5678);
    let mut text = reply(&mut rng, 7);
    text.push_str("\n\n");
    text.push_str(&reply(&mut rng, 8));
    text
}
