# Non-webview desktop UI options (2026): Slint, Iced, egui, Dioxus Native (Blitz), Makepad, Flutter desktop, Qt/QML, SwiftUI/AppKit, WinUI 3

Research date: 2026-10-11. GPUI (Zed) is out of scope and covered elsewhere.

Caveat up front: **there is no recent, independent benchmark that compares all of these frameworks on the same machine for idle RAM, binary size and startup time.** Most numbers below come from single-author posts, use different apps and platforms, and in places are vendor or SEO content. Treat them as orders of magnitude. If this decision depends on the numbers, a 1–2 day spike that builds the same "ticket board + 3 streaming logs + diff view" in the top 2–3 candidates and measures them on macOS and Windows is the only reliable way to settle it.

## 1. Idle RAM, binary size and startup time (measured numbers)

### Takeaway
The best recent measured data point comes from a single 2026 Linux case study: an egui (GPU) app used about 135 MB, and the same app ported to Slint used about 50 MB with the GPU renderer and about 30 MB with the software renderer. The GPU driver and context are a large share of baseline RAM for every GPU-rendered toolkit (egui, Iced, Makepad, Blitz, Flutter). The only head-to-head startup numbers are from 2023: egui and Iced both reached final render in roughly 200–330 ms on Linux. I found no trustworthy 2025–2026 measurements for Dioxus Native, Makepad, Flutter desktop, Qt Quick, SwiftUI or WinUI 3 idle RAM.

### Cited Findings
**Slint vs egui (best data, 2026-06-03, Linux, NVIDIA proprietary driver):**
- The "rproc" system monitor used ~135 MB RAM on egui. Ported to Slint, it used ~30 MB with the software renderer and ~50 MB with the GPU renderer, a 4.5x reduction. For comparison, Mission Center used 239 MB, Resources 200 MB and GNOME System Monitor 185 MB — [Trystan Sarrade, "How I took my Rust GUI from 135 MB to 30 MB by ditching the GPU" (2026-06-03)](https://trystan-sarrade.com/article/rust-gui-135mb-to-30mb-egui-to-slint/)
- The author attributes the GPU overhead to three things: the OpenGL/wgpu context mapped into the process, the graphics driver itself ("tens of MB of code and buffers" for NVIDIA), and the font/icon texture atlas. Quote: "for a 2D, mostly-idle desktop app, drawing on the CPU instead can be a 4.5x memory win" — [same source](https://trystan-sarrade.com/article/rust-gui-135mb-to-30mb-egui-to-slint/)
- Caveat: this is one app on one machine. Part of the 135 MB was app-specific (about 20 MB of NVIDIA monitoring libraries, per the article), so the comparison is not purely egui vs Slint.

**Tauri vs Iced vs egui (OLD: 2023-02, Ubuntu 20.04, X11, NVIDIA):**
- Binary size: Tauri 5 MB, Iced 17 MB, egui 18 MB — [Lukas Kalbertodt (2023-02-03)](http://lukaskalbertodt.github.io/2023/02/03/tauri-iced-egui-performance-comparison.html)
- Startup to final render: Tauri 366–417 ms, Iced 217–333 ms, egui 200–300 ms. Time until the window appears: egui immediate, Iced 33–50 ms, Tauri 100–150 ms — [same](http://lukaskalbertodt.github.io/2023/02/03/tauri-iced-egui-performance-comparison.html)
- Input lag: egui 2 frames, Iced 3 frames, Tauri 2–3 frames at 60 Hz. **Memory was not measured.** — [same](http://lukaskalbertodt.github.io/2023/02/03/tauri-iced-egui-performance-comparison.html)
- FLAG: these are Iced 0.7 and egui 0.20-era versions, now 3+ years old.

**Low-confidence aggregator figures (no stated methodology):**
- Claims include "egui ~3–5 MB binary, ~30–60 MB RAM" and "iced ~8–12 MB binary, ~40–100 MB RAM" — [an4t.com "Rust GUI Libraries Compared [2025]"](https://an4t.com/rust-gui-libraries-compared/). These appear to be SEO or possibly generated content, conflict with the 2023 measured binary sizes above, and should not be cited as measurements.

**Qt/QML:**
- Qt published a "Reducing binary size of Qt applications" series (part 3: 2025-06-09, Qt 6.10 dev, with fixes backported to 6.8/6.9) covering a minimal QML Window+Text app on Linux, Windows, macOS, wasm, iOS and Android. It found that size optimizations matter a lot on every platform. The actual numbers are in chart images that I could not extract — [Qt blog](https://www.qt.io/blog/reducing-binary-size-of-qt-applications-part-3-more-platforms)
- Older forum reports: a static Qt Widgets app is about 7–8 MB, and one Windows static main-window app was 13 MB. FLAG: these are old Qt 4/5-era threads — [Qt Forum](https://forum.qt.io/post/50874), [QtCentre](https://www.qtcentre.org/threads/50789-Minimal-size-of-a-statically-linked-Qt-application)

**Flutter desktop:**
- "A Flutter hello world application would be around 30 MB" — via search snippet from a [levelup.gitconnected article](https://levelup.gitconnected.com/macos-performance-comparison-flutter-desktop-vs-electron-6eb3c9c950de). The page returned 403 and I could not verify its date or method. Low confidence.

**WinUI 3:**
- Microsoft has acknowledged that WinUI 3 apps launch more slowly, use more RAM and have a larger install size than UWP. In May 2026 it announced WinUI optimizations for File Explorer: 41% fewer memory allocations, 45% fewer function calls and about 25% less time in WinUI code. It plans to make these default in WinUI and the Windows App SDK — [Techzine (2026-05-15)](https://www.techzine.eu/news/devops/141345/microsoft-promises-significant-speed-improvements-for-winui-3/); [PCWorld](https://www.pcworld.com/article/3199779/microsoft-admits-its-windows-11-apps-hog-ram-a-fix-is-coming.html)
- No absolute hello-world RAM number came from a primary source. A search snippet claimed a "20–100x RAM difference vs native C++", but I could not trace it to a measurement, so treat it as unverified.

**SwiftUI/AppKit:**
- I found no reliable 2025–2026 idle-RAM measurement. Search results surfaced old Apple Developer Forum threads about SwiftUI-on-macOS memory leaks ([thread 676860](https://developer.apple.com/forums/thread/676860)) and a vague "100–150 MB" figure that I could not attribute. Unverified.

### Inferences
- For a 2D, mostly-idle, text-heavy app, the choice of **GPU vs CPU renderer** probably moves idle RAM as much as the choice of framework does. Slint (software renderer, or Skia/FemtoVG) and Qt (raster) can avoid GPU-driver overhead. egui/eframe (wgpu or glow), Iced (wgpu, with a tiny-skia fallback), Makepad (Metal/DX11/GL), Blitz (wgpu+Vello) and Flutter (Impeller/Skia) pay for a GPU context.
- In practice, expect about 30–60 MB idle for Slint and lean Qt apps, about 50–150 MB for wgpu-based Rust apps (driver-dependent), and probably more for Flutter and WinUI 3 because of runtime overhead. These ranges are extrapolated from the cited data, not measured.
- Native SwiftUI/AppKit and WinUI 3 have "free" binary size because the system libraries are already on the machine. The tradeoff is two separate codebases.

### Gaps
- No measured idle RAM in 2025–2026 for Iced 0.14, egui 0.36, Dioxus Native 0.7/0.8, Makepad 1.0, Flutter 3.x desktop, Qt 6.8+ QML, SwiftUI on macOS 26, or WinUI 3 / Windows App SDK 1.8+/2.x.
- No cross-platform startup measurements newer than 2023.
- Exact numbers from the Qt binary-size blog were not extractable (they are images).

## 2. Maturity: versions, stability, production apps

### Takeaway
Iced, egui, Slint, Qt, Flutter and the native toolkits all have real production users. Slint and Qt are 1.x/6.x with formal release processes. egui and Iced are still 0.x but battle-tested (Rerun, COSMIC). Dioxus Native/Blitz is explicitly beta. Makepad reached 1.0 in 2025 and is already moving to a "2.0" architecture, with few production apps.

### Cited Findings
- **Iced**: the latest release is 0.14.0 (2025-12-07); 0.13 shipped 2024-09. Release cadence is about once a year — `gh release list -R iced-rs/iced` / [iced releases](https://github.com/iced-rs/iced/releases). 0.14 added reactive rendering, time-travel debugging, hot reloading, end-to-end testing, input method support and `table`/`grid` widgets. 0.13 added `rich_text` and `markdown`. 0.12 added the multi-line `TextEditor` and multi-window support — [iced releases](https://github.com/iced-rs/iced/releases)
- **COSMIC (System76) is built on Iced (libcosmic)**. Epoch 1 became stable and shipped with Pop!_OS 24.04 LTS on December 11, 2025 (one search summary said 2024; Pop!_OS 24.04 shipped in Dec 2025). Epoch 1.0.8 came out 2026-02-24, 1.1 added COSMIC Monitor (also Iced), and 1.4 is the latest — [Wikipedia: COSMIC](https://en.wikipedia.org/wiki/COSMIC_desktop); [Phoronix: Epoch 1.4](https://phoronix.com/news/COSMIC-Epoch-1.4); [OMG! Ubuntu (2026-06)](https://www.omgubuntu.co.uk/2026/06/cosmic-desktop-update-system-monitor)
- **egui**: the latest is 0.36.2 (2026-09-08), and 0.36.0 (2026-08-05) improved mobile keyboard support — `gh release list -R emilk/egui`. It is MIT/Apache-2.0, and the README names Rerun Viewer as a professional production user — [egui README](https://github.com/emilk/egui)
- **Slint**: the latest is 1.18.1 (2026-09-21), with roughly 2-month minor releases (1.14 in Oct 2025 through 1.18 in Sep 2026). It ships C++, Rust, Node.js and Python bindings, ESP-IDF and MCU templates — [Slint releases](https://github.com/slint-ui/slint/releases). Its main production footprint is embedded.
- **Dioxus Native / Blitz**: Dioxus 0.7 (2025-09-08) introduced Dioxus Native, which uses Blitz (Stylo CSS + Taffy layout + Parley text + Vello/wgpu + winit) — [Dioxus 0.7 blog](https://dioxuslabs.com/blog/release-070). The latest stable is 0.7.10 (2026-07-30), with 0.8.0-alpha.1 on 2026-07-31 — `gh release list -R DioxusLabs/dioxus`, crates.io `dioxus-native`. Blitz describes itself as **beta**: "usable for making apps if you are an early adopter and willing to live on the bleeding edge… still many bugs and missing features." Performance has not been the focus — [Blitz README](https://github.com/DioxusLabs/blitz); [Dioxus 0.7 blog](https://dioxuslabs.com/blog/release-070)
- **Makepad**: `makepad-widgets` 1.0.0 was published 2025-05-13 (crates.io). It is MIT licensed with about 7.2k stars, and renders through Metal, DX11, OpenGL and WebGL — [makepad GitHub](https://github.com/makepad/makepad). A "Makepad 2.0" effort moves from a compile-time DSL to runtime scripting, which is a migration risk — [makepad-2.0-migration skill page](https://skillselion.com/skills/zhanghandong/makepad-skills/makepad-2.0-migration) (secondary source). Its flagship app is the Robrix Matrix client (Project Robius) — [FOSDEM 2025 talk](https://ftp.fau.de/fosdem/2025/k4201/fosdem-2025-5841-robrix-a-pure-rust-multi-platform-matrix-client-and-more.vtt)
- **Qt**: currently on the 6.8 LTS / 6.9 / 6.10 line (see the Qt blog above). It is the most mature option here, with decades of desktop production use. I did not fetch new 2026 production examples.
- **WinUI 3**: Microsoft is betting on WinUI 3 for Windows 11's own shell apps, such as parts of File Explorer, and is actively optimizing it in 2026 — [Techzine](https://www.techzine.eu/news/devops/141345/microsoft-promises-significant-speed-improvements-for-winui-3/)

### Inferences
- Breaking-change risk is ordered roughly: Blitz/Dioxus Native > Makepad (2.0 shift) > Iced (yearly 0.x with breaking changes; COSMIC uses a fork via libcosmic) > egui (frequent 0.x minors, usually smooth to migrate) > Slint (semver 1.x) > Qt/native.

### Gaps
- I did not verify a list of non-embedded commercial desktop apps built on Slint, beyond Slint's own tools and showcases.
- Robrix and Moly production status and user counts were not verified.

## 3. Text quality, IME (Korean/CJK), accessibility, rich text, code views and terminals

### Takeaway
For a Korean-first user, **every Rust toolkit still has open IME bugs in 2026**, especially on Linux with fcitx5. Native toolkits, Qt and Flutter are the safest for IME. For code and diff views, Iced has a built-in `TextEditor` with highlighting support. egui has `TextEdit` plus syntect-based highlighting in `egui_extras`. Both Iced and egui have alacritty-backed terminal widgets (`iced_term`, `egui_term`). Slint gained `StyledText` and Markdown in 1.16 but has no code-editor or terminal component. Qt has QScintilla/KSyntaxHighlighting and QTermWidget/Konsole parts (not verified in this session).

### Cited Findings
**IME:**
- egui has open issues: on macOS, the 2-Set Korean IME's hanja candidate window cannot be opened with Option+Return ([#7974, opened 2026-03-13](https://github.com/emilk/egui/issues/7974)); on Linux X11 + Fcitx5, pre-edit text does not appear inside `TextEdit` ([#7975, 2026-03-13](https://github.com/emilk/egui/issues/7975)). The README credits IME support as a contribution — [egui README](https://github.com/emilk/egui)
- Iced: 0.14 added input method support ([releases](https://github.com/iced-rs/iced/releases)). Open issues: "text_input can't accept chinese input" ([#1801](https://github.com/iced-rs/iced/issues/1801), open since 2023) and CJK IME not working on WASM ([#2843](https://github.com/iced-rs/iced/issues/2843)). A fixed issue: the fcitx5 candidate window was misplaced on Linux ([#3258](https://github.com/iced-rs/iced/issues/3258), closed 2026)
- Slint, open issues: the last character is dropped when the window loses focus during IME composition in LineEdit ([#10861, 2026-02](https://github.com/slint-ui/slint/issues/10861)); TextInput/TextEdit is laggy and buggy on Wayland ([#10912, 2026-03](https://github.com/slint-ui/slint/issues/10912)); Chinese input question ([#4698](https://github.com/slint-ui/slint/issues/4698)); IME in the public Platform API ([#3811](https://github.com/slint-ui/slint/issues/3811))

**Accessibility:**
- egui integrates AccessKit for Windows and macOS (web is experimental) — [egui README](https://github.com/emilk/egui)
- Blitz/Dioxus Native includes AccessKit integration — [Blitz README](https://github.com/DioxusLabs/blitz)
- Slint has accessibility support, including custom actions since 1.6 — [Slint 1.6 blog](https://slint.dev/blog/slint-1.6-released)

**Rich text, code and terminals:**
- Iced has `TextEditor` (since 0.12), `rich_text` and `markdown` (0.13), and markdown quotes and task lists (0.14) — [iced releases](https://github.com/iced-rs/iced/releases)
- Slint 1.16 (2026-04-16) added a `StyledText` element and an `@markdown(...)` macro, switched glyph rasterization to swash, and added a `KeyBinding` element for shortcuts — [Slint 1.16 blog](https://slint.dev/blog/slint-1.16-released)
- `iced_term` (0.8) and `egui_term` are terminal widgets built on the alacritty_terminal backend. They support PTY, multiple instances, selection, scrolling and hyperlinks. COSMIC Terminal uses alacritty_terminal with cosmic-text and glyphon/wgpu rendering, plus a softbuffer/tiny-skia fallback — [iced_term docs.rs](https://docs.rs/crate/iced_term/latest); [egui_term docs.rs](https://docs.rs/crate/egui_term/latest); [COSMIC Terminal](https://rustutils.com/tools/cosmic-terminal/)
- COSMIC also ships cosmic-edit, a text editor built on Iced/libcosmic with cosmic-text (syntect highlighting). This is from memory, not fetched this session, so verify it.

### Inferences
- For "several logs streaming at once" plus diff and code views, a virtualized text view matters more than a rich-text widget. Iced's `TextEditor` and cosmic-text, and egui's `TextEdit` / custom galley painting, can do this. In Slint you would build it from a `ListView` with `StyledText` rows.
- If Korean input quality is a hard requirement, any Rust toolkit needs dedicated IME QA on macOS, Windows and Linux (fcitx5/ibus) before you commit.

### Gaps
- I did not verify that Makepad's built-in code editor is maintained, or check Makepad's IME and accessibility status.
- Flutter desktop, Qt, SwiftUI and WinUI IME and accessibility were not researched in depth this session. They are generally regarded as strong because they use OS text services, but that is not cited here.

## 4. Immediate vs retained mode: egui's CPU cost and repaint behavior

### Takeaway
egui is reactive by default, not a continuous game loop. It repaints only on input, on animation, or when code calls `ctx.request_repaint()`, and it is idle otherwise. When it does repaint, it rebuilds the whole UI, which the README puts at about 1–2 ms per frame. With several logs streaming at once, frequent `request_repaint` calls mean repeated full rebuilds, so you need to throttle them (for example, coalesce to 10–30 Hz). Iced 0.14 added "reactive rendering", and Slint is retained mode with property-dependency tracking that redraws only dirty regions.

### Cited Findings
- egui "only repaints when there is interaction or animation" and typically spends "1–2 ms per frame" on layout. It notes that "very little time has been spent optimizing". Immediate mode also limits layout because sizes are not known before widgets are laid out — [egui README](https://github.com/emilk/egui)
- Production pattern: objdiff (egui) removed per-frame repaint and switched to explicit `request_repaint()` calls from a file watcher and job threads, noting that full-rate repainting "is an enormous waste as the UI will be static most of the time" — [objdiff commit](https://git.axiodl.com/encounter/objdiff/commit/74e89130a8a4719e63d4e767bf5f098d183019c8)
- `Context::request_repaint` wakes the UI thread from background threads — [egui PR #1366](https://github-redirect.dependabot.com/emilk/egui/pull/1366)
- Iced 0.14 introduced "reactive rendering", which redraws only when needed — [iced releases](https://github.com/iced-rs/iced/releases)

### Inferences
- For this app, the cost driver in egui is the number of rebuilds per second times the size of the visible UI. Large logs need `ScrollArea::show_rows` virtualization, or every repaint re-lays out all the text.

### Gaps
- No measured CPU% numbers for egui vs Iced vs Slint under N concurrent streaming text panes.

## 5. Licensing

### Takeaway
egui, Iced, Makepad and Dioxus/Blitz are MIT/Apache (permissive). Slint is tri-licensed: GPLv3, royalty-free (free for proprietary desktop, mobile and web apps, with attribution conditions), or commercial (needed for embedded). Qt is LGPLv3 plus GPL for some modules, or commercial. Static linking under the LGPL pulls in relinking obligations, so a single static binary effectively pushes you toward a commercial license.

### Cited Findings
- egui: MIT OR Apache-2.0 — [egui README](https://github.com/emilk/egui). Makepad: MIT — [makepad GitHub](https://github.com/makepad/makepad)
- Slint: "GPL-3.0-only OR LicenseRef-Slint-Royalty-free OR LicenseRef-Slint-commercial". The Royalty-free License v2.0 covers proprietary desktop, mobile and web apps for free. Proprietary embedded requires a paid license — [Espressif component license page](https://components.espressif.com/components/slint/slint/versions/1.3.2/license); [Slint GitHub](https://github.com/slint-ui/slint). The royalty-free license's attribution requirement (an "AboutSlint" widget or a website notice) is from memory and not fetched; verify it.
- Qt: the primary open-source license is LGPLv3, and some modules are GPL-only. Dynamic linking lets app code stay proprietary, but you must provide the library source and allow users to relink. With static linking, "the application itself may no longer be 'work that uses the library' and thus become subject to LGPL". The commercial license is priced by quote — [Qt LGPL obligations](https://www.qt.io/licensing/open-source-lgpl-obligations)

### Gaps
- Current Qt commercial pricing for small businesses was not retrieved.

## 6. Dev velocity: hot reload, ecosystem, learning curve

### Takeaway
Slint has the best design-time tooling: the `.slint` DSL comes with an LSP and live preview, including visual color and gradient pickers since 1.11. Iced 0.14 added hot reloading and time-travel debugging. Dioxus has "subsecond" Rust hot-patching (0.7). Makepad has live-editable DSL. egui iterates fast because UI is plain Rust code, but it has no hot reload built in. Qt has QML live reload tooling and the largest component ecosystem. Native toolkits have Xcode/VS previews but need two codebases.

### Cited Findings
- Slint 1.11 added visual color and gradient pickers to Live-Preview — [Slint 1.11 blog](https://www.slint.dev/blog/slint-1.11-released). It ships slint-lsp and slint-viewer — [Slint releases](https://github.com/slint-ui/slint/releases)
- Iced 0.14 added hot reloading, time-travel debugging and end-to-end testing — [iced releases](https://github.com/iced-rs/iced/releases)
- Makepad has a "live-editable design language" and a Studio app — [makepad GitHub](https://github.com/makepad/makepad)
- Dioxus 0.7: Rust hot-patching ("subsecond") is from my prior knowledge. The fetched blog summary did not confirm it, so treat it as unverified.

### Gaps
- No quantitative comparison of learning curves. The Elm architecture (Iced) and immediate mode (egui) are commonly described as easy for Rust developers, but I did not source this.

## 7. Cross-platform coverage

### Takeaway
egui, Iced, Slint, Makepad, Dioxus Native, Flutter and Qt all target macOS, Windows and Linux. SwiftUI/AppKit is macOS only and WinUI 3 is Windows only, so a native approach means two or three UI codebases.

### Cited Findings
- Makepad: macOS (Metal), Windows (DX11), Linux (OpenGL), web — [makepad GitHub](https://github.com/makepad/makepad)
- Dioxus Native: desktop on macOS, Linux and Windows, plus iOS, Android and web — [Dioxus 0.7 blog](https://dioxuslabs.com/blog/release-070)
- Slint: C++ SDKs for Linux, Windows and macOS (x86-64 and arm64), plus Android — [Slint releases](https://github.com/slint-ui/slint/releases)
- Qt binary-size work was tested on Linux, Windows, macOS, iOS, Android and wasm — [Qt blog](https://www.qt.io/blog/reducing-binary-size-of-qt-applications-part-3-more-platforms)
- The 2023 Lukas benchmark warns that its results are Linux-only and may differ on Windows and macOS — [Lukas Kalbertodt](http://lukaskalbertodt.github.io/2023/02/03/tauri-iced-egui-performance-comparison.html)

### Inferences (overall fit for an AI-orchestrator app)
- **Lowest RAM with an acceptable feature set:** Slint with the software or Skia renderer. Weaknesses: no code-editor or terminal widget, and open IME bugs. You would build the diff/code view yourself.
- **Most ready-made pieces (editor, markdown, terminal, proven on a whole desktop):** Iced (via COSMIC's ecosystem: cosmic-text, iced_term). Costs: wgpu RAM overhead and yearly breaking changes.
- **Fastest to build developer-tool UIs:** egui (as in Rerun), with egui_term and syntect highlighting. Costs: GPU RAM overhead (~135 MB in one 2026 case), repaint throttling needed for streaming logs, and an open Korean IME bug on macOS.
- **Avoid for now:** Dioxus Native/Blitz (beta), and Makepad (2.0 churn and a small ecosystem).
- **Qt/QML:** mature and strong for IME. Costs: C++/QML, and LGPL relinking obligations or a commercial license.
- **Flutter / WinUI 3 / SwiftUI:** no measured evidence that any of them beats Slint or egui on RAM. WinUI 3 has acknowledged RAM and startup issues as of 2026. Native toolkits double the UI work.

### Gaps
- Wayland vs X11 behavior differences for each toolkit were not covered.
