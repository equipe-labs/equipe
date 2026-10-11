# Webview-based desktop frameworks: measured memory, size and performance (Tauri 2, Electron, Wails, Neutralino, Dioxus desktop), as of Oct 2026

Note on how good the sources are. There are few measured, reproducible numbers. The best sources are (1) the Elanis CI benchmark repo (GitHub Actions, all OSes, sums main and child processes), (2) the tauri-apps/tauri issue #5889 investigation, which accounts for shared memory (USS/PSS), (3) first-party app write-ups (Levminer/Authme, Superset). Many 2026 "Tauri vs Electron" blog posts (tech-insider.org, noqta.tn, dasroot.net, etc.) repeat figures with no traceable method. They are marked **UNVERIFIED** below.

## 1. Idle RAM and RAM under load, including helper processes

### Takeaway
On an empty app, Tauri/Wails (system webview) clearly beat Electron on macOS and Linux. On Windows (WebView2 = Chromium) the gap is small or reversed. Once real web content is loaded, total memory counting shared pages is roughly the same as Electron or worse: WKWebView/WebKitGTK used more than Electron in the one careful study. Memory is dominated by the web content, not the shell.

### Cited Findings
- **Elanis CI benchmark (page shows "Last Update: 10/2026"; GitHub Actions runners).** Two metrics: "process memory" = median of main process + all child processes; "system impact" = drop in system free memory. Empty app, values shown as Debug → Release (as read from the README tables) — [Elanis/web-to-desktop-framework-comparison](https://github.com/Elanis/web-to-desktop-framework-comparison)
  - Windows x64 process memory: Electron 365→279 MB; Tauri 454→307 MB; Wails 530→317 MB; Neutralino 548→487 MB; NW.js 227→461 MB. System impact: Electron 219→100 MB, Tauri 217→187 MB, Wails 367→206 MB, Neutralino 337→316 MB.
  - macOS arm64 process memory: Electron 460→364 MB; Tauri 231→96 MB; Wails 307→100 MB; Neutralino 181 MB. System impact: Electron 60→101 MB, Tauri −81→77 MB, Wails 200→77 MB.
  - Linux x64 process memory: Electron 676→589 MB; Tauri 553→94 MB; Wails 598→360 MB; Neutralino 914→820 MB. System impact: Electron 134→83 MB, Tauri 180→16 MB, Wails 253→58 MB.
  - Caveat: CI VMs with no GPU; the numbers move a lot between runs. An earlier (2025) snapshot of the same repo, quoted on GeekNews, gave single-window release numbers: Windows Electron ~93 MB / Tauri ~154 MB / Wails ~163 MB; macOS Tauri ~86 / Electron ~121 / Wails ~85 MB; Linux Tauri ~16 / Electron ~70 / Wails ~86 MB — [GeekNews 2025-05-29 citing the Elanis repo](https://news.hada.io/topic?id=21161). The two snapshots disagree in absolute terms. Both agree that **on Windows, Tauri/Wails are not lighter than Electron** and that on macOS and Linux Tauri is lighter for an empty app.
- **tauri-apps/tauri issue #5889 (shared-memory-aware study, ~2023, Tauri 1.x, still the most careful one):** 800×600 window, measured after 30–90 s idle. Loading **postman.com**: Tauri 421 MB (macOS), 581 MB (Ubuntu), 399 MB (Windows) vs Electron 337 / 240 / 318 MB. Loading **vscode.dev**: Tauri 429 / 572 / 370 MB vs Electron 332 / 222 / 312 MB. Default empty apps on Ubuntu: Electron USS 118 MB / PSS 207 MB; Tauri USS 125 MB / PSS 185 MB. Author's conclusion: earlier "Electron 2× Tauri" Linux figures came from not accounting for shared memory in Chromium's multi-process model. The maintainer called the benchmarks "smoke tests" and said "memory profiling is a more complex topic than numbers in a table usually conveys" — [tauri#5889](https://github.com/tauri-apps/tauri/issues/5889)
- **Levminer / Authme (real app ported Electron→Tauri; Windows 11, i5-4570, 16 GB; not dated in page; author calls it unscientific):** idle RAM Tauri ~80 MB vs Electron ~120 MB on Windows; Linux was described as "a big win for Tauri" (no figure in the extract); idle CPU 1%, GPU 0% for both — [levminer.com/blog/tauri-vs-electron](https://www.levminer.com/blog/tauri-vs-electron) (fetched via reader proxy; the direct fetch returned 403)
- **WebView2 process model (Microsoft docs):** each app gets its own WebView2 process group (1 browser process, 1+ renderer, GPU, utility/audio). There is "typically one renderer process for each WebView2 control". MS advises sharing one CoreWebView2Environment across controls, and notes that many instances "can strain system memory" — [MS Learn: process model](https://learn.microsoft.com/en-nz/Microsoft-edge/webview2/concepts/process-model), [MS Learn: performance](https://learn.microsoft.com/en-nz/Microsoft-edge/webview2/concepts/performance)
- **UNVERIFIED (marketing-grade aggregate):** "Tauri 2: 172 MB with 6 windows vs Electron 409 MB; idle 42 MB vs 168 MB". The article says these come from a "Nickel framework benchmark suite (Feb 2026)" and an "Open Web Foundation Desktop App Performance Tracker". It gives no links, hardware or OS, and I could not find either source — [tech-insider.org](https://tech-insider.org/tauri-vs-electron-2026/), repeated at [noqta.tn](https://noqta.tn/en/blog/tauri-2-desktop-apps-rust-web-technologies-2026)
- Commonly repeated "Tauri idle 30–50 MB vs Electron 150–300 MB" claims have no method attached (e.g., [dasroot.net 2026-03](https://dasroot.net/posts/2026/03/tauri-vs-electron-rust-cross-platform-apps/)). Treat as marketing.

### Inferences
- For our app (several streaming panes, diff views, maybe xterm), the web content (JS heap + DOM + GPU layers) will dominate. Expect any webview stack to sit at ~250–500 MB total under load unless the frontend is disciplined. Shell choice saves maybe ~50–150 MB on macOS/Linux and ~0 on Windows.
- Tauri's true total on macOS has to sum app + `WebContent` + `Networking` (+ GPU) processes. Activity Monitor's per-process view undercounts it. No 2024–2026 source I found does this summation for a real app with published numbers.
- Use a **single window/webview** with in-app panes rather than multiple OS windows. Each extra webview adds a renderer process (WebView2 documented; WKWebView behaves similarly).

### Gaps
- No 2024–2026 measured, helper-process-summed macOS Activity Monitor totals for a real Tauri 2 app found.
- No careful under-load benchmark (e.g., 10k-line log pane + editor) across frameworks.
- Neutralino and Dioxus-desktop have no measured RAM data outside the Elanis repo (Dioxus isn't in it at all).

## 2. Binary / installer size

### Takeaway
This is where system-webview stacks win clearly and reliably. Hello-world sizes: Neutralino ~2 MB, Tauri ~3–5 MB, Wails ~8–11 MB, Electron ~300+ MB unpacked (~85–150 MB installer).

### Cited Findings
- Elanis CI, empty-app build size: Windows Electron ≈385 MB, Tauri ≈3 MB, Wails ≈11 MB, Neutralino ≈2 MB, NW.js ≈563 MB, Flutter ≈25 MB. macOS arm64: Electron ≈320 MB, Tauri ≈5 MB, Wails ≈8 MB, Neutralino ≈2 MB. Linux: Electron ≈296 MB, Tauri ≈4 MB, Wails ≈9 MB, Neutralino ≈2 MB — [Elanis repo](https://github.com/Elanis/web-to-desktop-framework-comparison)
- Authme (real app) installer on Windows: Tauri ~2.5 MB vs Electron ~85 MB — [Levminer](https://www.levminer.com/blog/tauri-vs-electron)
- Desktop Docs (Mac, Electron→Tauri rewrite, 2025): app 1 GB → 172 MB, installer 232 MB → 69.5 MB. Most of the remaining size is bundled ML/media assets, not the framework — [GeekNews 2025-05-29](https://news.hada.io/topic?id=21161)
- Dioxus docs: desktop apps "typically under 5MB" because they use the system webview (doc claim, not a benchmark) — [Dioxus docs](https://dioxuslabs.com/learn/0.5/getting_started/desktop)
- DoltHub (2025-11-13) built Dolt Workbench in both: Electron hello world "up to 150 MB". Blockers for Tauri: no .appx/.msix, trouble with macOS universal binaries and codesigning, Node code must ship as a sidecar (which brings back size) — [DoltHub blog](https://www.dolthub.com/blog/2025-11-13-electron-vs-tauri/)
- **UNVERIFIED / likely false:** "TablePlus migrated from Electron to Tauri (18 MB vs 250 MB)". TablePlus is a native app. This came from an AI-search summary; do not use.

### Inferences
- If the orchestrator needs Node (e.g., to run agent CLIs), a Node sidecar adds roughly 40–100 MB and erodes Tauri's size advantage. A Rust/Go backend keeps it small.

### Gaps
- No dated 2026 measurements of the compressed installer sizes (.dmg/.msi) per framework from one source.

## 3. Startup time

### Takeaway
The data is noisy. In CI on GPU-less VMs, Electron started *fastest* for an empty app. Tauri on Linux CI was pathological (~30 s, likely a WebKitGTK/software-rendering issue). Real-device anecdotes favour Tauri (~2 s vs ~4 s).

### Cited Findings
- Elanis CI startup, Debug → Release: Windows Electron 530→153 ms, Tauri 4678→576 ms, Wails 6174→537 ms; macOS Electron 635→582 ms, Tauri 4863→1412 ms, Wails 4173→1396 ms; Linux Electron 339→199 ms, Tauri ~32 s→~30 s, Wails 3483→253 ms, Neutralino 518→332 ms — [Elanis repo](https://github.com/Elanis/web-to-desktop-framework-comparison)
- Authme: Tauri ~2 s vs Electron ~4 s, timed by hand on Windows — [Levminer](https://www.levminer.com/blog/tauri-vs-electron)
- UNVERIFIED: "cold start 380 ms vs 1,420 ms" — [tech-insider.org](https://tech-insider.org/tauri-vs-electron-2026/) (no source)

### Inferences
- Startup is not a deciding factor. All are sub-second to ~2 s on real hardware. Check Linux/WebKitGTK startup separately.

### Gaps
- No rigorous real-hardware startup comparison for 2025–2026.

## 4. Cross-platform rendering inconsistency

### Takeaway
Tauri/Wails/Dioxus/Neutralino ship three engines (WebView2/Chromium, WKWebView/Safari, WebKitGTK). WebKitGTK on Linux is the weak spot (GPU-driver issues, janky scrolling, slow WebGL/canvas). That matters for xterm.js (WebGL renderer) and large virtualized lists.

### Cited Findings
- Tauri's official doc "Linux Graphics Issues": on some setups, mostly NVIDIA, WebKitGTK and the driver disagree, giving "anything from a blank window to subtle rendering problems". WebGL/canvas "can silently land on a slow path", causing high input latency and low FPS. The doc recommends non-WebGL fallbacks on Linux — [v2.tauri.app/develop/debug/linux-graphics](https://v2.tauri.app/develop/debug/linux-graphics/)
- WebKitGTK scrolling bugs: slow/janky scrolling on Tweetdeck in Epiphany web apps (Intel/radeon, while Firefox was smooth) — [WebKit bug 220085](https://bugs.webkit.org/show_bug.cgi?id=220085); slow scrolling of a site front page — [WebKit bug 250988](https://bugs.webkit.org/show_bug.cgi?id=250988)
- Elanis CI shows a ~30 s Tauri startup on Linux, consistent with WebKitGTK slow paths in headless or GPU-less environments — [Elanis repo](https://github.com/Elanis/web-to-desktop-framework-comparison)
- tauri#5889: WebKit-based webviews used >90 MB more than Electron on real web apps across platforms — [tauri#5889](https://github.com/tauri-apps/tauri/issues/5889)

### Inferences
- For xterm.js, plan on the DOM or canvas renderer on Linux/WebKit and WebGL only on Chromium. Test Safari-specific CSS and API gaps (e.g., WKWebView quirks) in CI.
- Electron gives one engine everywhere, so behaviour and performance are predictable. That consistency is the main thing it buys for its size cost.

### Gaps
- No quantitative 2025–2026 frame-rate benchmarks of WebKitGTK vs WebView2 vs WKWebView for log/terminal-heavy UIs.

## 5. Known heavy points (webviews, JS heap, editors, terminals, DOM)

### Takeaway
The biggest controllable cost is the frontend: terminals (xterm WebGL contexts and scrollback), editors and unvirtualized logs. Superset, an Electron multi-agent coding orchestrator very close to our app, halved its JS heap by limiting live terminals.

### Cited Findings
- **Superset (Electron agent orchestrator with many terminals), changelog 2026-07-19:** cut active terminal GPU contexts from 16 to 4 and offloaded hidden terminals to disk. JS heap went from 581 MB to 286 MB (−51%) and renderer memory from 1,127 MB to 838 MB (−26%). Moving git and file-watch work to worker pools cut the worst UI stall from 82.3 to 28.0 ms. One shared port scan replaced per-terminal scans, taking process spawns from 15 to 1 per cycle — [superset.sh changelog](https://superset.sh/changelog/2026-07-19-performance-memory-and-load)
- Terminal scrollback cost, general: a 10,000-line, 80-column Unicode buffer is ~3.2 MB of raw cells; urxvt grew ~16 MB for 10k lines — [dev.to discussion](https://dev.to/ben/how-much-of-a-memory-impact-do-tabs-in-my-terminal-have-114l) (not xterm.js-specific)
- WebView2: one renderer per control; share the environment to reduce memory — [MS Learn](https://learn.microsoft.com/en-nz/Microsoft-edge/webview2/concepts/performance)

### Inferences
- Do these regardless of framework: one webview; virtualized log panes with ring buffers; cap xterm scrollback; at most a few live WebGL terminals (others detached or serialized); CodeMirror 6 rather than Monaco (Monaco is much heavier — no measured source found here); a lightweight UI framework.
- Superset's ~840 MB renderer after optimization shows that a feature-rich orchestrator can reach close to 1 GB even on Electron. The shell is a small share of the total.

### Gaps
- No measured per-instance memory for Monaco vs CodeMirror 6 vs xterm.js in 2024–2026 sources found here.
- No measured DOM-node-count vs memory data for long logs.

## 6. Real-world apps and counter-examples

### Takeaway
Real ports show large size wins and modest RAM wins on Windows (Authme 80 vs 120 MB). Careful measurement of real web content shows Tauri equal to or worse than Electron (tauri#5889). A lightweight result depends on the frontend.

### Cited Findings
- Authme: Tauri ~80 MB vs Electron ~120 MB idle on Windows; installer 2.5 MB vs 85 MB — [Levminer](https://www.levminer.com/blog/tauri-vs-electron)
- Desktop Docs: size 1 GB → 172 MB, installer 232 → 69.5 MB, faster indexing (much of that from the Rust rewrite) — [GeekNews](https://news.hada.io/topic?id=21161)
- Counter-example: postman.com and vscode.dev loaded in Tauri used 370–581 MB vs Electron 222–337 MB on all three OSes — [tauri#5889](https://github.com/tauri-apps/tauri/issues/5889)
- Counter-example: the Elanis Windows CI shows Tauri/Wails at or above Electron for an empty app — [Elanis repo](https://github.com/Elanis/web-to-desktop-framework-comparison)
- DoltHub evaluated Tauri but stayed on Electron because of packaging gaps (msix, universal binaries, Node sidecar) — [DoltHub](https://www.dolthub.com/blog/2025-11-13-electron-vs-tauri/)

### Inferences
- Ranking for "small install": Neutralino ≈ Tauri ≈ Dioxus < Wails << Electron. Ranking for "low RAM": Tauri ≈ Wails ≈ Dioxus (same system webviews) < Electron on macOS/Linux; roughly equal on Windows. Neutralino measured worst in Elanis CI (possibly its separate server process plus the webview).
- Dioxus desktop uses the same wry/tao webview layer as Tauri, so its memory should match Tauri's. That is inferred, not measured.

### Gaps
- No measured Dioxus desktop RAM numbers.
- No independent RAM numbers for well-known Tauri 2 apps (e.g., by name) found in this pass.
