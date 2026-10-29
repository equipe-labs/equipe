//! The fake daemon's Markdown torture test: say `torture 1` … `torture 4` to a
//! member and it answers with that part — reasoning, tool steps with their
//! outcomes (output, exit codes, an edit's change), and replies that use
//! everything a conversation draws: headings, lists, tasks, links and file
//! paths, callouts, code in many languages, a long block, a diff, a wide
//! table, CSV, math, a Mermaid diagram, images. `torture 6` is a PM's busy
//! turn — long shell output, calls one after another, a failure, an edit,
//! a comment, a subagent, news; `torture 7` holds a command running for a
//! few seconds, to see a call under way; `torture 8` is a PM's long answer in
//! Korean — paragraphs, a list, bold, ticket links — to read for wrapping and
//! the measure.

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
    Event::ToolUse { session, tool: tool.into(), target: Some(target.into()), label: None, background: false }
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
        6 => activity(session, &events, seconds).await,
        8 => stream(session, KOREAN, &events, seconds).await,
        7 => {
            send(tool(session, "Read", &format!("{SITE}/src/main.rs")));
            pause(100).await;
            send(done(session, "Read", &format!("{SITE}/src/main.rs"), ok(30)));
            send(tool(session, "Bash", "cargo build --release -p equipe-desktop"));
            pause(6000).await;
            send(done(session, "Bash", "cargo build --release -p equipe-desktop", ToolOutcome { ok: true, exit: Some(0), ms: Some(6000), output: "   Compiling equipe-desktop v0.38.1
    Finished `release` profile [optimized] target(s) in 6.02s".into(), ..Default::default() }));
            stream(session, "빌드가 끝났습니다.", &events, seconds).await;
        }
        _ => {
            write_picture();
            stream(session, PART4, &events, seconds).await;
        }
    }
    let usage = Usage { input_tokens: Some(1200), output_tokens: Some(340), context_tokens: Some(420), context_window: Some(1000), ..Default::default() };
    send(Event::TurnEnded { session, outcome: TurnOutcome::Completed, usage: Some(usage) });
}

const PS: &str = "/usr/local/bin/equipe daemon --squad demo";

/// A PM's busy turn, as the person saw it: a wall of shell output, reads, a
/// search, a failing test, an edit, a comment on a ticket, a subagent, then
/// news from the board and the reply.
async fn activity(session: SessionId, events: &broadcast::Sender<Event>, seconds: f32) {
    let send = |e: Event| {
        let _ = events.send(e);
    };
    for thought in ["데몬이 두 번 떠 있는지 보고, ", "T-47 진행 상황을 확인하겠습니다."] {
        send(Event::Thinking { session, text: thought.into() });
        pause(500).await;
    }
    stream(session, "데몬 상태부터 확인해 보겠습니다.", events, seconds / 4.).await;
    let ps = "ps -axo pid,lstart,command | grep -E 'equipe (daemon|agent)' | grep -v grep";
    send(tool(session, "Bash", ps));
    pause(150).await;
    let mut lines: Vec<String> = (0..24).map(|n| format!("{} Wed Oct 15 22:{:02}:11 2026 {PS} --member m{n}", 4100 + n * 7, n)).collect();
    lines.push("4321 Wed Oct 15 23:01:52 2026 claude -p --append-system-prompt You are the PM of squad demo. You plan work as tickets, hand it to members, review what they hand in…".into());
    lines.push("4388 Wed Oct 15 23:02:03 2026 equipe agent --squad demo --member backend".into());
    let (output, skipped) = ToolOutcome::tail(&lines.join("\n"));
    send(done(session, "Bash", ps, ToolOutcome { ok: true, exit: Some(0), ms: Some(300), output, skipped, diff: None }));
    for file in ["daemon.log", "src/runtime.rs"] {
        send(tool(session, "Read", &format!("{SITE}/{file}")));
        pause(80).await;
        send(done(session, "Read", &format!("{SITE}/{file}"), ok(25)));
    }
    send(tool(session, "Grep", "fn brief_pm"));
    pause(80).await;
    send(done(session, "Grep", "fn brief_pm", ok(60)));
    send(tool(session, "Bash", "cargo test -p equipe-core runtime::brief"));
    pause(300).await;
    let failure = "running 4 tests\ntest runtime::brief::once ... ok\ntest runtime::brief::twice ... FAILED\n\nfailures:\n---- runtime::brief::twice stdout ----\nthread 'runtime::brief::twice' panicked at crates/core/src/runtime.rs:2501:9:\nassertion `left == right` failed\n  left: 2\n right: 1\n\nerror: test failed, to rerun pass `-p equipe-core --lib`";
    let (output, skipped) = ToolOutcome::tail(failure);
    send(done(session, "Bash", "cargo test -p equipe-core runtime::brief", ToolOutcome { ok: false, exit: Some(101), ms: Some(8_200), output, skipped, diff: None }));
    send(tool(session, "Edit", &format!("{SITE}/src/runtime.rs")));
    pause(100).await;
    let diff = format!("diff --git a/{SITE}/src/runtime.rs b/{SITE}/src/runtime.rs\n--- a\n+++ b\n@@ -2496,7 +2496,8 @@\n         let shown = self.said(news, &args);\n-        self.brief_pm(&squad, briefing, Some(shown));\n-        self.brief_pm(&squad, briefing, None);\n+        if self.pm_of(&squad) != Some(id) {{\n+            self.brief_pm(&squad, briefing, Some(shown));\n+        }}\n     }}\n");
    send(done(session, "Edit", &format!("{SITE}/src/runtime.rs"), ToolOutcome { ok: true, ms: Some(70), diff: Some(diff), ..Default::default() }));
    send(tool(session, "Bash", "cargo test -p equipe-core runtime::brief"));
    pause(200).await;
    send(done(session, "Bash", "cargo test -p equipe-core runtime::brief", ToolOutcome { ok: true, exit: Some(0), ms: Some(7_900), output: "running 4 tests\ntest result: ok. 4 passed; 0 failed".into(), ..Default::default() }));
    let comment = "T-47 · PM: 갤러리 스와이프는 좋습니다. 확대했을 때 이미지가 화면 밖으로 밀리는 문제만 고쳐 주세요 — 모바일 Safari에서 재현됩니다. 고친 뒤 스크린샷 두 장(확대 전/후)을 붙여 주세요.";
    send(tool(session, "mcp__equipe__comment", comment));
    pause(80).await;
    send(done(session, "mcp__equipe__comment", comment, ok(40)));
    send(tool(session, "Task", "사이드바 접기: 토글 위치 고정"));
    pause(200).await;
    send(done(session, "Task", "사이드바 접기: 토글 위치 고정", ok(42_000)));
    for news in [
        "backend handed in T-71 for your review · 요청 수 제한 미들웨어.",
        "You accepted T-58 · 앱 아이콘 후보 세 가지.",
        "designer이(가) T-64을(를) 제안했습니다: 빈 화면 일러스트 세트",
        "frontend이(가) T-42을(를) 검토해 달라고 넘겼습니다 · 체크아웃 페이지 결제 수단 선택.",
    ] {
        send(Event::Notice { session, text: news.into() });
    }
    stream(session, ACTIVITY, events, seconds).await;
}

const ACTIVITY: &str = "## 정리\n\n데몬은 하나만 떠 있고, PM 브리핑이 두 번 가던 문제는 `src/runtime.rs`에서 고쳤습니다. 로그는 `daemon.log`에 있고, 설정 이름은 `brief_pm` 그대로입니다.\n\n### 바뀐 점\n\n1. PM이 자기 일을 넘길 때는 브리핑을 보내지 않습니다\n2. 테스트 `runtime::brief::twice`가 이제 통과합니다 — 한 번 실패한 뒤 고쳐서 다시 돌렸고, 네 개 모두 통과했습니다\n\n---\n\n**T-47**에는 확대 문제를 고쳐 달라고 댓글을 남겼습니다.";

/// A PM's long answer in Korean, as one came to the person: words that broke
/// between syllables at the end of a line ("프로젝 / 트", "지 / 시하기").
const KOREAN: &str = "좋은 질문이에요. 지금 구조에서는 사이드바에서 프로젝트를 누르면 PM이 그 프로젝트 이야기로 대화를 이어 가고, 다른 프로젝트 이야기는 접힌 줄로 따로 모입니다. 그래서 프로젝트마다 대화방을 따로 만들 필요는 없어요.

## 하고 싶은 것

말씀하신 건 **매번 지시하지 않아도 PM이 알아서 다음 일을 고르는 흐름**을 만들고 싶은데, 그게 지금 어디까지 되는지 궁금하신 거죠? 정리하면 이렇습니다.

- 보드에 할 일이 쌓여 있으면 PM이 우선순위대로 하나씩 멤버에게 넘깁니다. 넘길 때마다 대화에 짧게 알려 드려요.
- 멤버가 일을 끝내면 [T-119](ticket:119)처럼 **검토할 일**로 올라오고, 받은편지함에서 바로 수락하거나 변경을 요청할 수 있습니다.
- 결정이 필요한 일만 여러분 차례로 남고, 나머지는 기다리지 않고 계속 진행됩니다.

### 아직 안 되는 것

다만 두 가지는 아직 손으로 하셔야 합니다. 첫째, 새 프로젝트의 방향을 정하는 일은 PM이 제안까지만 하고 확정은 여러분이 해 주셔야 하고, 둘째, 배포처럼 되돌리기 어려운 일은 항상 허락을 받고 진행합니다. Internationalization처럼 아주 긴 영어 단어나 https://example.com/very/long/path/that/keeps/going/without/any/spaces/at/all 같은 주소는 줄 끝에서 어쩔 수 없이 잘릴 수 있어요.

원하시면 **T-120**으로 \"자동으로 다음 일 고르기\" 설정을 만들어서, 켜 두면 PM이 하루에 한 번 보드를 훑고 할 일을 정리하도록 해 둘게요. 어떻게 할까요?";

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
