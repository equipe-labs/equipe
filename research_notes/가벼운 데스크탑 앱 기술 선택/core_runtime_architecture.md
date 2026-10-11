# Core Runtime Cost and Memory Architecture for a Lightweight equipe Desktop App (2026)

Note on method: the web had few primary, recent, apples-to-apples numbers for idle RSS, so I also took **local measurements** on the requester's machine (Apple Silicon arm64, macOS 26.2, measured 2026-10-11). Numbers marked "[local]" come from `ps -o rss` (RSS) and `footprint <pid>` (phys_footprint) about 2–8 seconds after process start. They come from one machine with n=1, so read them as rough sizes, not benchmarks. Scripts were in the session scratchpad, and the main commands are reproduced inline. On macOS, RSS counts shared, clean, file-backed pages (dylibs, mapped binary). phys_footprint counts dirty, compressed, and swapped private memory, which is what Activity Monitor shows. That is why the two differ by 2–3x.

## A1. JS runtimes: idle memory and binary size (Node, Node SEA, Bun compile, Deno compile)

### Takeaway
At idle, Bun is about half of Node: ~18–20 MB RSS / ~6 MB phys_footprint for Bun vs ~40 MB RSS / ~12 MB footprint for Node. Every JS single-binary option still ships a 55–100 MB executable, because the whole JS engine is embedded. A JS core costs roughly 6–15 MB of real footprint at idle and 60–90 MB of disk. That is small next to the agent CLIs it spawns (see A4).

### Cited Findings
- [local] Node v24.19.0 (Homebrew), idle `setInterval` script: **RSS 40.6 MB, phys_footprint 12 MB**. `node -e 'process.memoryUsage()'` reports heapUsed 3.6 MB. `/usr/bin/time -l node -e ''`: max RSS 42.3 MB, peak footprint 12.0 MB. Node 22 was not measured, but it is expected to be similar (inference).
- [local] Bun 1.3.9, same idle script: **RSS 20.1 MB, phys_footprint 6.4 MB**.
- [local] `bun build --compile` of that one-line script produced a **60.3 MB** executable (darwin-arm64). Running it idle: **RSS 18.2 MB, phys_footprint 6.1 MB**.
- [local] The Homebrew node binary is a 50 KB stub linked to `libnode.137.dylib`, which is **66.7 MB**. A Node SEA has to embed the full node binary, so expect roughly 65–100 MB per platform before app code.
- Official Node Windows x64 binaries: v22.15.0 ≈ 84 MB, v22.21.0 ≈ 86 MB, v25.0.0 ≈ 92 MB. These set the baseline size of a Node SEA, which is the node binary plus an injected blob. — [nodejs.org dist v22.15.0](https://nodejs.org/download/release/v22.15.0/win-x64); [r2.nodejs.org v25.0.0](https://r2.nodejs.org/dist/v25.0.0/win-x64)
- A real-world Node SEA (a malware sample) came to 81 MB as a standalone binary. — [SafeDep](https://safedep.io/ti/ttps/node-js-single-executable-application-packaging)
- Deno 1.41 roughly halved `deno compile` output: a Hello World on Mac ARM went from 116 MB (1.40) to 58 MB (1.41). — [Deno 1.41 blog](https://deno.com/blog/v1.41)
- An unverified claim says a 2025–26 experimental Deno QuickJS backend cuts a compiled Hello World from ~65 MiB to ~35.6 MiB. — [besthub.dev (secondary, low-quality aggregator; not verified against deno.com)](https://www.besthub.dev/articles/deno-s-big-shift-quickjs-replaces-v8-cutting-compile-size-by-almost-45-dcbacaae56b6). **Flag: treat as unconfirmed.**
- Bun v1.4.1 made JavaScriptCore drop JIT code after long idle periods. Its benchmark (Linux x64, after 60 s of load then 3 min idle) reports RSS for Express of **53 MB in Bun 1.4.1 vs 83 MB in Node v26**, and for Fastify 55 vs 89 MB. — [Bun v1.4.1 blog](https://bun.com/blog/bun-v1.4.1) (vendor benchmark; seen via search snippet, not fetched in full)
- Claude Code itself ships as a single native binary of **223.8 MB** (`~/.local/bin/claude` 2.1.285, Mach-O arm64) [local]. This suggests (not confirmed) that it is a Bun-compiled bundle, and it shows how large a compiled JS app can get.

### Inferences
- For equipe's core, moving from Node to Bun saves about 6 MB of footprint and 20 MB of RSS at idle. That is real but small. The bigger cost of JS is the 60–90 MB of disk per platform for a separate sidecar binary.
- If the desktop shell is Electron, it already ships Node, so the core can run in Electron's utility/main process with no extra disk cost. If the shell is Tauri or native, a JS sidecar adds about 60–90 MB to the install, which may be bigger than the rest of the app.
- better-sqlite3 is a native addon and works with Node SEA. Bun has a built-in `bun:sqlite`, and Node 22.5+ has a built-in `node:sqlite`. Either removes the native-addon packaging problem (inference; the API differs from better-sqlite3).

### Gaps
- No primary, current (2026) benchmark of Node 22 SEA size per OS was found. Numbers here come from official binary sizes and local libnode.
- Deno was not installed locally, so I have no measured idle RSS for Deno. The Deno 2.x compile size for 2026 was not confirmed from a primary source.

## A2. Porting the core to Rust: daemon RSS, and the state of the Rust ACP crate / how Zed drives agents

### Takeaway
A Rust tokio + rusqlite daemon is typically a few MB of footprint at idle. I found no authoritative published figure. The closest data point is that the Rust Codex `app-server` sits at 48 MB footprint, but it does far more. The official Rust ACP crate is mature and moves fast (v3.3.0, Oct 2026, 133 dependent crates). Zed runs every external agent as a subprocess speaking ACP over stdio, through adapters.

### Cited Findings
- No web source gave a measured idle RSS for a minimal tokio daemon. Search turned up only daemon code that tunes tokio (for example 1 worker thread and 8 MiB stack) and trims allocator memory, because "Rust's default allocator doesn't compact … RSS only shrinks when the allocator hands pages back". — [rmux server_runtime.rs](https://docs.rs/crate/rmux/latest/source/src/server_runtime.rs); [leankg gc.rs](https://docs.rs/crate/leankg/0.19.31/source/src/gc.rs)
- [local] `codex app-server` (codex-cli 0.154.0, Rust, stdio JSON-RPC) idle with no session: **RSS 104 MB, phys_footprint 48 MB**. The codex binary is **222.7 MB** on disk. This gives an upper bound for a "big" Rust agent process, not a minimal daemon.
- Crate `agent-client-protocol` (crates.io): latest **3.3.0, published 2026-10-09**. Versions 3.0.0 (10-06), 3.1.0 (10-07), 3.2.0 (10-08), 2.2.0 (09-18), 2.1.0 (09-04) show very fast churn, including a major bump. It has ~5.1 M total downloads and ~2.0 M recent, with **133 reverse dependencies**. — [crates.io API](https://crates.io/api/v1/crates/agent-client-protocol) (queried 2026-10-11)
- The Rust SDK repo `agentclientprotocol/rust-sdk` (218 stars) provides a core protocol implementation for clients, agents, proxies and "conductors". It also has HTTP/SSE and WebSocket transports (`agent-client-protocol-http`), MCP integration via `agent-client-protocol-rmcp`, and both stable v1 and experimental v2 protocol support. — [GitHub rust-sdk](https://github.com/agentclientprotocol/rust-sdk)
- Zed: "External Agents run as separate processes that communicate with Zed over ACP." "Zed hosts the thread in the Agent Panel and Threads Sidebar, while the External Agent usually owns its own runtime, auth, model selection, tools, and native configuration." Agents are installed through the **ACP Registry** (`zed: acp registry`), and `dev: open acp logs` shows the traffic. — [Zed docs: External Agents](https://zed.dev/docs/ai/external-agents)
- Zed runs the Claude Agent SDK, which runs Claude Code under the hood, through a dedicated ACP adapter. Codex uses a separate `codex-acp` adapter that wraps the Codex CLI. — [Zed docs](https://zed.dev/docs/ai/external-agents); [codex-acp summary](https://www.sourcepulse.org/projects/21679199)
- npm adapter versions (queried 2026-10-11): `@agentclientprotocol/claude-agent-acp` **0.89.1** depends on `@anthropic-ai/claude-agent-sdk` 0.3.293, `@agentclientprotocol/sdk` 1.8.0, and zod 4. `@zed-industries/codex-acp` is at **0.16.0**. — [npm registry claude-agent-acp](https://registry.npmjs.org/@agentclientprotocol/claude-agent-acp/latest); [npm registry codex-acp](https://registry.npmjs.org/@zed-industries/codex-acp/latest)

### Inferences
- A Rust core would likely idle at about 3–10 MB phys_footprint (tokio current-thread plus rusqlite with default 2 MB cache). That would save roughly 5–10 MB of footprint compared with Node or Bun, and 60–90 MB of disk compared with a JS sidecar. **Unverified estimate.** It should be measured with a tiny prototype; cargo was not installed locally.
- The Rust ACP crate is usable. Because it shipped 3 majors' worth of churn in about one month, version pinning will be needed.
- The Claude adapter path chains Node, which is bigger than either of the other runtimes: claude-agent-acp (Node) → Claude Agent SDK → spawns the `claude` binary. So equipe-in-Rust only shrinks equipe's own process. The Claude ACP path still costs a Node adapter **plus** a Claude Code process.

### Gaps
- No measured RSS of a minimal tokio + rusqlite binary was found or taken. Recommend a 30-minute prototype measurement.
- I did not confirm from source code that Zed's own client uses the `agent-client-protocol` crate. It is very likely, since the crate originated at Zed, but that is unverified here.
- The memory of the claude-agent-acp Node adapter process itself was not measured.

## A3. better-sqlite3 vs rusqlite memory; SQLite page cache settings

### Takeaway
Both bindings wrap the same SQLite C library. SQLite's own memory is governed by `cache_size`, which defaults to about 2 MB per connection, plus optional mmap. The binding choice matters little; data layout matters a lot. Keeping logs in SQLite on disk and paging them in cut footprint from 131 MB (logs as a JS array) to 36 MB [local].

### Cited Findings
- "The default suggested cache size is -2000, which means the cache size is limited to 2048000 bytes of memory." Negative N means about abs(N×1024) bytes. The default can be changed with `SQLITE_DEFAULT_CACHE_SIZE`. — [SQLite PRAGMA docs](https://www.sqlite.org/pragma.html)
- `PRAGMA mmap_size`: "maximum number of bytes that are set aside for memory-mapped I/O … If N is zero then memory mapped I/O is disabled." `temp_store=MEMORY` keeps temp tables and indices in RAM. `soft_heap_limit` / `hard_heap_limit` cap SQLite's heap, and the pragma "can only lower the heap limit". — [SQLite PRAGMA docs](https://www.sqlite.org/pragma.html)
- [local] Node 24 holding 200k log lines (~90 bytes each) as JS objects: **RSS 130.6 MB, heapUsed 69.2 MB, phys_footprint 131 MB**.
- [local] The same 200k lines in `node:sqlite` `:memory:` DB: **RSS 90.7 MB, phys_footprint 59 MB**.
- [local] The same 200k lines in a **file-backed** `node:sqlite` DB (WAL, cache_size=-2000), with only a 200-row page loaded: **RSS 66.9 MB, phys_footprint 36 MB**. The DB file was 20.5 MB. The remaining footprint is mostly the Node baseline plus insert-time allocations not yet returned to the OS.

### Inferences
- Choosing better-sqlite3 vs rusqlite vs node:sqlite vs bun:sqlite is not a memory decision. They all link SQLite, and per-connection cost is about the cache size plus a few hundred KB (inference). Pick based on packaging: rusqlite with the `bundled` feature compiles SQLite in, and node:sqlite/bun:sqlite avoid a native addon.
- Recommended settings for a log store: `journal_mode=WAL`, keep `cache_size` at the default or set it explicitly (-2000 to -8000), `mmap_size=0` (or small) if footprint optics matter, and `temp_store=FILE`. Optionally set `hard_heap_limit` as a safety cap. mmap pages appear as file-backed RSS, not phys_footprint, which confuses RSS-based budgets (inference).

### Gaps
- No published head-to-head memory benchmark of better-sqlite3 vs rusqlite was found.

## A4. Memory used by the spawned agent CLIs (the system-footprint floor)

### Takeaway
The agent processes dwarf any core runtime. An idle Claude Code is about 190 MB RSS / 92 MB footprint, and an idle Codex app-server is about 104 MB RSS / 48 MB footprint [local]. Claude Code has a long history of reported leaks into multiple GB. With 3 agents running, the floor is roughly 150–300 MB of footprint before equipe's UI, so a 10–30 MB difference between UI frameworks is a minor share.

### Cited Findings
- [local] `claude` 2.1.285 interactive in a PTY, idle about 8 s, no prompt: **RSS 192 MB, phys_footprint 92 MB**. This is the user's real config, which may include MCP servers and hooks. No child processes were seen in its process group at that moment.
- [local] `codex app-server` (0.154.0) idle: **RSS 104 MB, phys_footprint 48 MB**.
- GitHub issue mirrors (claudeissues.com is an **aggregator** of anthropics/claude-code issues; I did not check each one on github.com) report: RSS growing from ~500 MB to 4.6 GB+ within minutes ([#46931](https://claudeissues.com/issue/46931-bug-memory-leak-claude-code-rss-grows-from-500mb-to-4-6gb-within-minutes)); ~3 GB within seconds of startup ([#17650](https://claudeissues.com/issue/17650-bug-memory-leak-claude-code-consumes-3gb-ram-within-seconds-of-startup)); about +500 MB/hour, reaching ~4 GB after 3 h ([#37240](https://claudeissues.com/issue/37240-bug-memory-usage-increases-during-extended-sessions)); and 2.4→8.4 GB in 2 min on v2.1.80 ([#37195](https://claudeissues.com/issue/37195-bug-memory-leak-process-grows-from-2-4gb-to-8-4gb-in-2-minutes-with-minimal-tool)). Many of these are marked resolved.
- One summary claims that as of Claude Code v2.1.215 the install-time RSS dropped to about 200 MB (from 11 GB). — [claudeissues #79421](https://claudeissues.com/issue/79421-bug-high-memory-usage-during-claude-install-sh) (secondary)
- A third-party blog warns about runaway child processes (for example, test runners spawned by Claude Code) pushing usage to about 20 GB. — [aident.ai](https://aident.ai/blog/fix-claude-code-runaway-child-oom) (vendor blog, low confidence)
- OpenAI rewrote Codex CLI in Rust partly for "no runtime garbage collection, resulting in lower memory consumption". — [devclass, 2025-06](https://www.devclass.com/ai-ml/2025/06/02/nodejs-frustrating-and-inefficient-openai-rewrites-ai-coding-tool-in-rust/1619589)
- The Claude ACP path adds a Node adapter process (`claude-agent-acp` → Claude Agent SDK) on top of the `claude` process. — [npm registry](https://registry.npmjs.org/@agentclientprotocol/claude-agent-acp/latest); [Zed docs](https://zed.dev/docs/ai/external-agents)

### Inferences
- Total system footprint with 1 Claude + 1 Codex + 1 Gemini (Gemini CLI is Node; not measured): roughly ≥ 200 MB footprint / ≥ 400 MB RSS at idle, and often far more mid-session. A Tauri vs Electron gap of about 50–150 MB therefore matters for the empty-app first impression, but not for the steady state once agents are running.
- equipe should show per-agent memory in its UI and could add a kill or restart policy (for example, restart an agent when footprint exceeds a threshold between turns), because the biggest real-world memory risk is a leaking agent child, not the UI.
- Spawn agents lazily and shut down idle agents, rather than keeping a warm pool.

### Gaps
- Gemini CLI memory was not measured because it is not installed. The claude-agent-acp adapter process memory was not measured. Agent memory during an active turn was not measured, since that would spend tokens.

## B1. Long streaming logs: virtualization, ring buffers, persist-to-SQLite with lazy paging

### Takeaway
Never hold the full transcript or log in UI memory. Keep a bounded ring buffer (for example, the last 2–5k lines) for the live tail, write everything to SQLite or disk, and render with a virtualized list that fetches pages by id range. Locally this cut footprint about 3.6x for 200k lines.

### Cited Findings
- [local] 200k lines as JS objects = 131 MB footprint, vs 36 MB with file-backed SQLite and a 200-row page (see A3).
- xterm.js moved to a typed-array buffer that "recycles memory". With 1000 lines of scrollback it uses 8–15 MB JS heap + 1.5 MB C++ heap, vs 25–50 MB JS heap + 6 MB with the old JS-array buffer. — [xterm.js buffer performance issue (lightrun mirror)](https://lightrun.com/answers/xtermjs-xterm-js-buffer-performance-improvements)
- Alacritty used to allocate the full scrollback up front. It now grows the grid in chunks of 1000 rows, starting with only the visible area. Default history is 10,000 lines. — [Alacritty source/config mirror](https://git.causa-arcana.com/PolytreeDE/alacritty/commit/c1e831dab03bd1a48d2aa0f383aa5aaea2b3632b); [itsfoss](https://itsfoss.com/alacritty-terminal/)

### Inferences
- Pattern: the core appends ACP `session/update` chunks to SQLite (batched in transactions every ~50–100 ms). The UI receives only a "new rows up to id N" notification and the visible window. When the view is scrolled up, older rows come from `WHERE id < ? ORDER BY id DESC LIMIT 200`.
- Coalesce streaming token deltas into one row per message or tool call, not one row per token, to keep row counts and render work low.
- Use virtualized lists with variable heights in the web UI (for example TanStack Virtual) or the native-UI equivalent (GPUI `uniform_list` / `list` in Zed). I found no 2026 primary source with benchmarks, so this is inference.

### Gaps
- No engineering blog from Warp, Linear, or VS Code with concrete before/after memory numbers for log virtualization was found in this pass. Warp's GitHub issues show regressions (for example "~100MB to 500MB+" after an Oct-2025 update) but no design write-up. — [Warp issue #7938 mirror](https://upd.dev/warpdotdev/warp/issues/7938)

## B2. Single- vs multi-process; lazy-loading views and syntax highlighters

### Takeaway
For memory, a single UI process plus one core process (or in-process core) is best. Each extra process costs its runtime baseline (Node ≈12 MB footprint, a webview renderer much more). The agent CLIs are unavoidable separate processes anyway. Load heavy subsystems (diff viewer, syntax grammars, markdown renderer) only when they are first used.

### Cited Findings
- Zed keeps agents as separate processes but hosts every thread in one app UI ("Zed hosts the thread in the Agent Panel and Threads Sidebar"). — [Zed docs](https://zed.dev/docs/ai/external-agents)
- [local] Each extra Node process costs about 12 MB footprint / 40 MB RSS, and each Bun process about 6 MB / 20 MB (A1).

### Inferences
- Recommended layout: UI process + equipe core (Rust in-process inside a Tauri/native shell, or a single sidecar) + N agent processes spawned lazily. Avoid one helper process per pane or session.
- Load tree-sitter or other highlighters per language on first use. Better still, highlight only visible diff hunks and cache tokens, not full files. Shiki/TextMate grammars in a webview should be loaded with dynamic import per language. Not verified with numbers in this pass.

### Gaps
- No primary source with numbers for tree-sitter grammar memory or WASM grammar size per language was retrieved in this pass.

## B3. Avoiding per-pane webviews and terminal emulators; xterm.js vs alacritty_terminal

### Takeaway
Render agent output as structured ACP events (messages, tool calls, diffs) in one list. Do not embed a terminal emulator per agent pane. If a terminal is needed, use one shared one, cap scrollback, and only create it when the user opens it.

### Cited Findings
- A 160x24 xterm.js terminal with 5000 lines of scrollback filled uses about 34 MB. — [xterm.js buffer issue (lightrun mirror)](https://lightrun.com/answers/xtermjs-xterm-js-buffer-performance-improvements) (older measurement, date uncertain)
- xterm.js typed-array buffer: 8–15 MB JS heap for 1000-line scrollback (same source).
- alacritty_terminal grows its grid lazily in 1000-row chunks, defaulting to 10k history. — [Alacritty commit mirror](https://git.causa-arcana.com/PolytreeDE/alacritty/commit/c1e831dab03bd1a48d2aa0f383aa5aaea2b3632b)

### Inferences
- With ACP, agents already emit structured events, so a terminal emulator is only needed for agent-run shell commands (ACP `terminal/*` methods). Store their output as text in SQLite and render it as plain monospace in the virtualized list, adding ANSI color parsing only for visible rows.
- Each extra webview (WKWebView/WebView2) is a separate content process costing tens of MB (inference, not measured here). One webview with internal routing beats one webview per pane.

### Gaps
- No 2025–26 primary measurement of alacritty_terminal bytes per cell or line was found. My recollection is that a cell is about 24 bytes (char + fg/bg + flags), but this is **unverified** and not included as a finding.

## B4. Measuring memory correctly and a memory budget in CI

### Takeaway
On macOS, use **phys_footprint** (what Activity Monitor and jetsam use), not RSS. RSS here was 2–3.5x higher because of shared and dylib pages (Node 40.6 vs 12 MB, Claude 192 vs 92 MB). Sum it across the whole process tree (UI + webview helpers + core + agents). Gate CI on the peak and idle footprint of a scripted scenario.

### Cited Findings
- phys_footprint "is the metric macOS uses for its own memory accounting"; "unlike RSS, it counts compressed and swapped-out anonymous pages", and RSS "reads artificially low exactly when the process is heaviest". — [basemind sysres.rs (docs.rs)](https://docs.rs/crate/basemind/0.23.1/source/src/sysres.rs)
- Chromium reads both `resident_size` and `phys_footprint` from `task_info(TASK_VM_INFO)` on Mac. — [Chromium process_metrics_mac.cc](https://chromium.googlesource.com/chromium/src/+/72.0.3608.5/base/process/process_metrics_mac.cc)
- The macOS `footprint` command inspects process memory (phys_footprint and its breakdown). — [unixtutorial: footprint](https://www.unixtutorial.org/commands/footprint/)
- [local] `/usr/bin/time -l <cmd>` on macOS prints both "maximum resident set size" and "**peak memory footprint**". For `node -e ''` that was 42.3 MB vs 12.0 MB, which makes it an easy CI hook.

### Inferences
- CI budget recipe:
  1. Launch the app with a fixture session (mock ACP agent that streams N=50k lines).
  2. After warm-up, sample phys_footprint of the whole tree (`footprint -p`, or `proc_pid_rusage(RUSAGE_INFO_V4).ri_phys_footprint`) at idle and at peak.
  3. Fail if above budget (for example, idle < 60 MB, peak < 150 MB, excluding real agent processes).
  4. On Linux, use PSS from `/proc/<pid>/smaps_rollup`. On Windows, use Private Working Set or commit.
  5. Track the trend per commit.
- Use a **mock ACP agent** in CI so budgets measure equipe itself and are not mixed with agent-CLI memory, which is outside equipe's control.

### Gaps
- No Zed, VS Code, or Linear engineering post on CI memory budgets was retrieved in this pass. Windows and Linux equivalents above come from general knowledge and were not cited.
