//! The fake daemon's Markdown torture test: say `torture 1` … `torture 4` to a
//! member and it answers with that part — reasoning, tool steps with their
//! outcomes (output, exit codes, an edit's change), and replies that use
//! everything a conversation draws: headings, lists, tasks, links and file
//! paths, callouts, code in many languages, a long block, a diff, a wide
//! table, CSV, math, a Mermaid diagram, images. `torture 6` is a PM's busy
//! turn — long shell output, calls one after another, a failure, an edit,
//! a comment, a subagent, news; `torture 7` holds a command running for a
//! few seconds, to see a call under way; `torture 8` is a PM's long answer —
//! paragraphs, a list, bold, ticket links — to read for wrapping and the
//! measure.

use std::time::Duration;

use equipe_core::protocol::{Event, SessionId, ToolOutcome, TurnOutcome, Usage};
use tokio::sync::broadcast;

const SITE: &str = "/tmp/fake/site";

const PART1: &str = r#"## Header clean-up

**Summary**: I tidied the header component; `cargo test` failed once, then passed. The details are on [T-12](ticket:12).

### Files changed

- `src/components/Header.tsx:42` — logo alignment
- `crates/desktop/src/views/chat.rs` and `Cargo.toml`
  - A nested item: `README.md` links tidied
    - A deeper item with *italics* and ~~strikethrough~~
- An outside link: [GPUI docs](https://www.gpui.rs) · a file link: [main.rs](/tmp/fake/site/src/main.rs:12)

### To do

- [x] Align the header
- [x] Check IME input
- [ ] Check dark-mode contrast

> [!NOTE]
> This change is for the **desktop app** only. The `equipe` CLI is unchanged.

> [!WARNING]
> Run `npm run build` once more before deploying.

> A plain quote looks as it is.

Done."#;

const PART2: &str = r#"Some code examples.

```rust
/// Draws the conversation.
pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement {
    let total: u32 = (1..=10).filter(|n| n % 2 == 0).sum();
    println!("total: {total}");
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
    """The nth Fibonacci number."""
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

const PART4: &str = r#"A wide table, math and a diagram.

| Screen | Status | Owner | Last change | Note | Branch | Tests | Reviewer |
|---|---|---|---|---|---|---|---|
| Header | In progress | frontend | 2026-10-15 14:02 | Logo alignment and menu spacing | feat/header-align | Pass (128) | pm |
| Composer | Waiting | frontend | 2026-10-14 09:30 | Enter during an IME composition | fix/ime-enter | Fail (2) | qa |
| Daemon link | Done | backend | 2026-10-13 18:45 | SSH reconnect backoff | feat/ssh-backoff | Pass (64) | pm |

```csv
name,role,"note, with a comma"
pm,lead,"owns ticket ""T-12"""
frontend,screens,header
```

Inline math $E = mc^2$ and $\alpha + \beta$, while money like $5 and $10 stays as written.

$$
\int_0^1 x^2 \, dx = \frac{1}{3}
$$

```mermaid
flowchart LR
    A[Person] --> B{PM}
    B -->|ticket| C[frontend]
    B -->|ticket| D[backend]
    C --> E[Review]
    D --> E
```

A local image:

![Screen capture](/tmp/fake/site/shot.png)

An image of a missing file: ![Remote file](/Users/nobody/missing.png)
"#;

const PART5: &str = r#"Math and diagrams on their own:

Inline $\sqrt{a^2 + b^2}$ and $x_i$.

$$
\sum_{k=1}^{n} k = \frac{n(n+1)}{2}
$$

```mermaid
sequenceDiagram
    Person->>PM: Fix the header
    PM->>frontend: T-12
    frontend-->>PM: Asks for review
```

```mermaid
flowchart LR
    A[Person] --> B{PM}
    B -->|ticket| C[frontend]
    B -->|ticket| D[backend]
```
"#;

fn long_block() -> String {
    let mut out = String::from("A long code block shows its first 24 lines only.\n\n```rust\n");
    for n in 1..=40 {
        out.push_str(&format!("let line_{n} = compute({n}); // the computation goes on a line at a time, and a very long line has to wrap once it is wider than the screen {n}\n"));
    }
    out.push_str("```\n\nPress below to see it all.");
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
            for thought in ["I'll look at the header first, ", "check why the test fails, ", "then fix it."] {
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
        8 => stream(session, LONG, &events, seconds).await,
        7 => {
            send(tool(session, "Read", &format!("{SITE}/src/main.rs")));
            pause(100).await;
            send(done(session, "Read", &format!("{SITE}/src/main.rs"), ok(30)));
            send(tool(session, "Bash", "cargo build --release -p equipe-desktop"));
            pause(6000).await;
            send(done(session, "Bash", "cargo build --release -p equipe-desktop", ToolOutcome { ok: true, exit: Some(0), ms: Some(6000), output: "   Compiling equipe-desktop v0.38.1
    Finished `release` profile [optimized] target(s) in 6.02s".into(), ..Default::default() }));
            stream(session, "The build is done.", &events, seconds).await;
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
    for thought in ["I'll see whether the daemon is running twice, ", "then check how T-47 is going."] {
        send(Event::Thinking { session, text: thought.into() });
        pause(500).await;
    }
    stream(session, "Checking the daemon first.", events, seconds / 4.).await;
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
    let comment = "T-47 · PM: The gallery swipe is good. Please fix just the image being pushed off the screen when zoomed — it reproduces in mobile Safari. Once fixed, attach two screenshots (before and after zooming).";
    send(tool(session, "mcp__equipe__comment", comment));
    pause(80).await;
    send(done(session, "mcp__equipe__comment", comment, ok(40)));
    send(tool(session, "Task", "Collapse the sidebar: pin the toggle's position"));
    pause(200).await;
    send(done(session, "Task", "Collapse the sidebar: pin the toggle's position", ok(42_000)));
    for news in [
        "backend handed in T-71 for your review · Rate-limit middleware.",
        "You accepted T-58 · Three app icon candidates.",
        "designer proposed T-64: Empty-state illustration set",
        "frontend handed in T-42 for your review · Checkout page payment method choice.",
    ] {
        send(Event::Notice { undo: None, session, text: news.into() });
    }
    stream(session, ACTIVITY, events, seconds).await;
}

const ACTIVITY: &str = "## Summary\n\nOnly one daemon is running, and the PM's briefing going out twice is fixed in `src/runtime.rs`. The log is in `daemon.log`, and the setting is still called `brief_pm`.\n\n### What changed\n\n1. The PM sends no briefing when it hands over its own work\n2. The test `runtime::brief::twice` passes now — it failed once, was fixed and run again, and all four pass\n\n---\n\nI left a comment on **T-47** asking for the zoom problem to be fixed.";

/// A PM's long answer, as one came to the person: paragraphs, a list, bold
/// and ticket links, a long word and an address with no spaces in it.
const LONG: &str = "Good question. As things stand, clicking a project in the sidebar has the PM carry the conversation on about that project, and talk about other projects gathers on its own folded lines. So there's no need for a separate conversation per project.

## What you want

What you describe is **a flow where the PM picks the next piece of work by itself, without being told each time**, and you'd like to know how far that works today — right? In short:

- When work piles up on the board, the PM hands it to members one at a time, by priority, and tells you briefly in the conversation each time.
- When a member finishes, it comes up as **something to review**, like [T-119](ticket:119), and you can accept it or ask for changes straight from the inbox.
- Only what needs a decision is left as your turn; everything else carries on without waiting.

### What doesn't work yet

Two things are still yours to do by hand. First, the PM only proposes a new project's direction — settling it is up to you; second, anything hard to undo, like a deploy, always waits for your yes. A very long word like Internationalization or an address like https://example.com/very/long/path/that/keeps/going/without/any/spaces/at/all may have to be cut at the end of a line.

If you like, I'll make a setting for \"pick the next work automatically\" as **T-120**: switched on, the PM looks over the board once a day and sorts out what to do. Shall I?";

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
