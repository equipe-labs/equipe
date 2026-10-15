//! The fake daemon's Markdown torture test: say `torture 1` … `torture 4` to a
//! member and it answers with that part — reasoning, tool steps with their
//! outcomes (output, exit codes, an edit's change), and replies that use
//! everything a conversation draws: headings, lists, tasks, links and file
//! paths, callouts, code in many languages, a long block, a diff, a wide
//! table, CSV, math, a Mermaid diagram, images.

use std::time::Duration;

use equipe_core::protocol::{Event, SessionId, ToolOutcome, TurnOutcome, Usage};
use tokio::sync::broadcast;

const SITE: &str = "/tmp/fake/site";

const PART1: &str = r#"## 헤더 정리 결과

**요약**: 헤더 컴포넌트를 정리했고 `cargo test`가 한 번 실패한 뒤 통과했습니다. 자세한 건 [T-12](ticket:12)에 남겼습니다.

### 바뀐 파일

- `src/components/Header.tsx:42` — 로고 정렬
- `crates/desktop/src/views/chat.rs` 와 `Cargo.toml`
  - 중첩된 항목: `README.md` 링크 정리
    - 더 깊은 항목 *기울임*과 ~~취소선~~
- 바깥 링크: [GPUI 문서](https://www.gpui.rs) · 파일 링크: [main.rs](/tmp/fake/site/src/main.rs:12)

### 할 일

- [x] 헤더 정렬
- [x] 한글 입력 확인
- [ ] 다크 모드 대비 점검

> [!NOTE]
> 이 변경은 **데스크톱 앱**에만 해당합니다. `equipe` CLI는 그대로입니다.

> [!WARNING]
> 배포 전에 `npm run build`를 한 번 더 돌려 주세요.

> 평범한 인용문은 그대로 보입니다.

끝."#;

const PART2: &str = r#"코드 예시입니다.

```rust
/// 대화를 그립니다.
pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement {
    let total: u32 = (1..=10).filter(|n| n % 2 == 0).sum();
    println!("합계: {total}");
}
```

```tsx
export function Header({ title }: { title: string }) {
  const [open, setOpen] = useState(false);
  return <header className="flex gap-2">{title}</header>;
}
```

```python
def fib(n: int) -> int:
    """n번째 피보나치 수."""
    return n if n < 2 else fib(n - 1) + fib(n - 2)
```

```shell
$ cargo build -p equipe-desktop --features snapshot
$ EQUIPE_SCRIPT="wait:1500;shot:/tmp/a.png" ./target/debug/equipe-desktop
```

```json
{"name":"equipe","version":"0.9.0","private":true,"scripts":{"dev":"vite","build":"tsc && vite build"},"tags":["a","b"],"empty":{}}
```

```diff
diff --git a/src/a.rs b/src/a.rs
@@ -1,3 +1,3 @@
 fn main() {
-    println!("old");
+    println!("new");
 }
```
"#;

const PART4: &str = r#"넓은 표와 수식, 다이어그램입니다.

| 화면 | 상태 | 담당 | 마지막 변경 | 메모 | 브랜치 | 테스트 | 리뷰어 |
|---|---|---|---|---|---|---|---|
| 헤더 | 진행 중 | frontend | 2026-10-15 14:02 | 로고 정렬과 메뉴 간격 조정 | feat/header-align | 통과 (128) | pm |
| 입력창 | 대기 | frontend | 2026-10-14 09:30 | 한글 조합 중 Enter 처리 | fix/ime-enter | 실패 (2) | qa |
| 데몬 연결 | 완료 | backend | 2026-10-13 18:45 | SSH 재연결 백오프 | feat/ssh-backoff | 통과 (64) | pm |

```csv
이름,역할,"메모, 쉼표 포함"
pm,관리,"티켓 ""T-12"" 담당"
frontend,화면,헤더
```

인라인 수식 $E = mc^2$ 와 $\alpha + \beta$, 그리고 돈 $5와 $10은 그대로입니다.

$$
\int_0^1 x^2 \, dx = \frac{1}{3}
$$

```mermaid
flowchart LR
    A[사람] --> B{PM}
    B -->|티켓| C[frontend]
    B -->|티켓| D[backend]
    C --> E[리뷰]
    D --> E
```

로컬 이미지:

![화면 캡처](/tmp/fake/site/shot.png)

없는 파일의 이미지: ![원격 파일](/Users/nobody/missing.png)
"#;

const PART5: &str = r#"수식과 다이어그램만 따로:

인라인 $\sqrt{a^2 + b^2}$ 그리고 $x_i$.

$$
\sum_{k=1}^{n} k = \frac{n(n+1)}{2}
$$

```mermaid
sequenceDiagram
    사람->>PM: 헤더 고쳐 줘
    PM->>frontend: T-12
    frontend-->>PM: 리뷰 요청
```

```mermaid
flowchart LR
    A[사람] --> B{PM}
    B -->|티켓| C[frontend]
    B -->|티켓| D[backend]
```
"#;

fn long_block() -> String {
    let mut out = String::from("긴 코드 블록은 처음 24줄만 보입니다.\n\n```rust\n");
    for n in 1..=40 {
        out.push_str(&format!("let line_{n} = compute({n}); // 한 줄씩 늘어나는 계산이 이어지고, 아주 긴 줄은 화면 폭을 넘으면 줄바꿈되어야 합니다 {n}\n"));
    }
    out.push_str("```\n\n끝까지 보려면 아래를 누르세요.");
    out
}

fn tool(session: SessionId, tool: &str, target: &str) -> Event {
    Event::ToolUse { session, tool: tool.into(), target: Some(target.into()) }
}

fn done(session: SessionId, tool: &str, target: &str, outcome: ToolOutcome) -> Event {
    Event::ToolResult { session, tool: tool.into(), target: Some(target.into()), outcome }
}

fn ok(ms: u64) -> ToolOutcome {
    ToolOutcome { ok: true, ms: Some(ms), ..Default::default() }
}

async fn stream(session: SessionId, text: &str, events: &broadcast::Sender<Event>, seconds: f32) {
    let chars: Vec<char> = text.chars().collect();
    let chunks = chars.chunks(6).count().max(1);
    let gap = Duration::from_secs_f32(seconds / chunks as f32);
    for chunk in chars.chunks(6) {
        let _ = events.send(Event::TextDelta { session, text: chunk.iter().collect() });
        tokio::time::sleep(gap).await;
    }
}

async fn pause(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

pub async fn turn(session: SessionId, part: usize, events: broadcast::Sender<Event>, seconds: f32) {
    let send = |e: Event| {
        let _ = events.send(e);
    };
    pause(150).await;
    match part {
        1 => {
            for thought in ["헤더를 먼저 보고, ", "테스트가 왜 실패하는지 확인한 뒤 ", "고치겠습니다."] {
                send(Event::Thinking { session, text: thought.into() });
                pause(400).await;
            }
            send(tool(session, "Read", &format!("{SITE}/src/components/Header.tsx")));
            send(tool(session, "Read", &format!("{SITE}/src/styles.css")));
            pause(100).await;
            send(done(session, "Read", &format!("{SITE}/src/components/Header.tsx"), ok(40)));
            send(done(session, "Read", &format!("{SITE}/src/styles.css"), ok(35)));
            send(tool(session, "Bash", "cargo test -p equipe-core"));
            pause(300).await;
            let failure: String = (1..=30).map(|n| format!("test tests::case_{n} ... ok\n")).collect::<String>()
                + "test tests::header_aligns ... FAILED\n\nfailures:\n    tests::header_aligns\n\nerror: test failed, to rerun pass `-p equipe-core --lib`";
            let (output, skipped) = ToolOutcome::tail(&failure);
            send(done(session, "Bash", "cargo test -p equipe-core", ToolOutcome { ok: false, exit: Some(101), ms: Some(12_400), output, skipped, diff: None }));
            send(tool(session, "Edit", &format!("{SITE}/src/components/Header.tsx")));
            pause(100).await;
            let diff = format!(
                "diff --git a/{SITE}/src/components/Header.tsx b/{SITE}/src/components/Header.tsx\n--- a\n+++ b\n@@ -38,9 +38,10 @@\n export function Header() {{\n   const logo = useLogo();\n   return (\n-    <header className=\"flex\">\n+    <header className=\"flex items-center gap-3\">\n+      <Logo src={{logo}} />\n       <Nav />\n-      <Menu align=\"left\" />\n+      <Menu align=\"end\" />\n     </header>\n   );\n }}\n@@ -80,2 +81,2 @@\n-const GAP = 8;\n+const GAP = 12;\n"
            );
            send(done(session, "Edit", &format!("{SITE}/src/components/Header.tsx"), ToolOutcome { ok: true, ms: Some(80), diff: Some(diff), ..Default::default() }));
            send(tool(session, "Bash", "cargo test -p equipe-core"));
            pause(250).await;
            send(done(session, "Bash", "cargo test -p equipe-core", ToolOutcome { ok: true, exit: Some(0), ms: Some(9_800), output: "running 31 tests\ntest result: ok. 31 passed; 0 failed".into(), ..Default::default() }));
            send(tool(session, "Bash", "npm run build"));
            pause(200).await;
            stream(session, PART1, &events, seconds).await;
        }
        2 => stream(session, PART2, &events, seconds).await,
        3 => stream(session, &long_block(), &events, seconds).await,
        5 => stream(session, PART5, &events, seconds).await,
        _ => {
            write_picture();
            stream(session, PART4, &events, seconds).await;
        }
    }
    let usage = Usage { input_tokens: Some(1200), output_tokens: Some(340), context_tokens: Some(420), context_window: Some(1000), ..Default::default() };
    send(Event::TurnEnded { session, outcome: TurnOutcome::Completed, usage: Some(usage) });
}

/// A picture for the reply to show: a soft gradient with a frame.
fn write_picture() {
    let (w, h) = (480u32, 220u32);
    let mut img = image::RgbaImage::new(w, h);
    for (x, y, px) in img.enumerate_pixels_mut() {
        let t = x as f32 / w as f32;
        let s = y as f32 / h as f32;
        let edge = x < 4 || y < 4 || x >= w - 4 || y >= h - 4;
        *px = if edge { image::Rgba([40, 60, 90, 255]) } else { image::Rgba([(120. + 100. * t) as u8, (150. + 60. * s) as u8, 210, 255]) };
    }
    let _ = std::fs::create_dir_all(SITE);
    let _ = img.save(format!("{SITE}/shot.png"));
}
