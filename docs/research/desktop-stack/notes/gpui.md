# GPUI (Zed's GPU-accelerated Rust UI framework) as a candidate for the equipe desktop app (as of Oct 2026)

Research date: 2026-10-11. No Rust toolchain or Zed install was available on the research machine, so **no numbers below were measured by me**. Every number is attributed to its source and labeled "measured by X" or "claim".

## Standalone use outside Zed: crates.io, releases, API stability, docs, official support

### Takeaway
GPUI *can* be used outside Zed, but in practice it is not a well-supported standalone library. The official `gpui` crate on crates.io has not been updated since Oct 2025 (0.2.2). The upstream README on `main` now refers to a `gpui_platform` crate that is not on crates.io, so real users follow git `main` or a third-party snapshot or fork. Zed's maintainers put Zed's needs first and send out-of-scope features to community forks. The result is a split ecosystem: gpui-ce, GPUI Box, Longbridge's `gpui-pre-*` snapshots and GPUI Fast, and these are not type-compatible with each other.

### Cited Findings
- crates.io `gpui` versions: 0.1.0 (2022-06-23, a placeholder), then 0.2.0 (2025-10-09), 0.2.1 (2025-10-14), 0.2.2 (2025-10-22). Nothing newer. About 348k total downloads, about 195k recent — [crates.io API](https://crates.io/api/v1/crates/gpui)
- The upstream `crates/gpui/README.md` on main (fetched 2026-10-11) says GPUI "is still in active development as we work on the Zed code editor, and is still pre-1.0. There will often be breaking changes between versions. You'll also need to use the latest version of stable Rust." It tells users to add both `gpui` and `gpui_platform` — [zed repo, crates/gpui/README.md](https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md)
- `gpui_platform` does not exist on crates.io (API returns "crate `gpui_platform` does not exist", checked 2026-10-11) — [crates.io API](https://crates.io/api/v1/crates/gpui_platform). My inference: the published crate lags main by about a year.
- The README does include "Ownership and data flow" and "Accessibility" guide pages and explains the three "registers": Entities, Views/Render, and Elements — [crates/gpui/README.md](https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md)
- A third-party guide dated 2026-10-01 reports that an app moving from the Oct-2025 crate to mid-2026 needed only "four one-line edits": `Application::new()` signature, `Menu` gaining a `disabled` field, and `window.focus()` needing `cx`. So the API churn was small for that one app — [baeseokjae blog, 2026-10-01](https://baeseokjae.github.io/posts/native-app-for-coding-agents-rust-gpui/)
- Same source: about 177k GPUI downloads and about 95k gpui-component downloads in the 90 days before Oct 2026 — [baeseokjae blog](https://baeseokjae.github.io/posts/native-app-for-coding-agents-rust-gpui/)
- A gpui-component discussion (2025-12-25) says upstream GPUI is "de-emphasizing the needs of GPUI users outside of Zed". Features not usable in Zed, such as custom shaders and tray support, get rejected, and Zed developers point people to gpui-ce instead. At that time gpui-ce was "381 commits behind mainline" — [longbridge/gpui-component discussion #1856](https://github.com/longbridge/gpui-component/discussions/1856)
- gpui-ce (Community Edition) latest crate is 0.2.2 (2026-08-28) after 0.3.x (2025-12-27). Its version numbering is odd. About 13.6k total downloads — [crates.io API gpui-ce](https://crates.io/api/v1/crates/gpui-ce); fork description at [codeberg temportalflux/gpui](https://codeberg.org/temportalflux/gpui)
- GPUI Box (0.1.1, 2026-08-12) is "an independent distribution of GPUI… not an official Zed project". It warns that "Rust treats types from separate GPUI packages as distinct", so you cannot mix forks. Requires Rust 1.97+ — [docs.rs gpui-box](https://docs.rs/crate/gpui-box/0.1.1)
- Longbridge publishes `gpui-pre-*` snapshot crates of upstream GPUI, and its component library pins and re-exports a matching GPUI release — [gpui-component/GPUI Kit README](https://github.com/longbridge/gpui-component)
- Longbridge also started "GPUI Fast" (repo created 2026-09-24, 187 stars, v0.1.0 on crates.io, "experimental"). It is a performance fork: "GPUI extracted from the Zed monorepo as a standalone workspace" — [longbridge/gpui-fast](https://github.com/longbridge/gpui-fast)
- Zed itself ships often: latest release v1.23.2 on 2026-10-07, and the repo has about 91.6k stars — [GitHub API zed releases](https://github.com/zed-industries/zed/releases)

### Inferences
- "Official support for external users" is best-effort at most. Zed welcomes standalone use in the README but is clearly not product-managing GPUI for third parties. A small team would most likely pin a git revision or a Longbridge `gpui-pre` snapshot through gpui-component, then upgrade on their own schedule.
- The fork split (gpui-ce, GPUI Box, GPUI Fast, gpui-pre) is a real long-term risk. Picking a dependency line is a lock-in decision.

### Gaps
- I found no formal statement from Zed Industries such as "GPUI is supported for third parties" or a roadmap to 1.0. I also found no official explanation for why crates.io publishing stopped after Oct 2025.
- I did not assess docs.rs coverage or the number and quality of upstream examples in detail. The README references `examples/` such as `a11y`, and issue #64389 shows `examples/input.rs` exists but has an IME bug.

## Platform support: macOS (Metal), Linux (wgpu, formerly Blade), Windows (DirectX 11)

### Takeaway
macOS/Metal is the most mature backend. Windows moved to DirectX 11 in mid-2025. Linux moved from Blade to wgpu in Feb 2026, and that renderer still has a long list of open crash and compatibility issues and PRs. Linux Wayland in particular still has regressions, such as GNOME 50.

### Cited Findings
- macOS: "Rendering uses Metal and is always available, but glyph rasterization needs `font-kit`". Building needs full Xcode installed — [crates/gpui/README.md](https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md)
- Windows: "Windowing uses Win32 and text uses DirectWrite" — [README](https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md). Renderer: "windows: Port to DirectX 11" merged 2025-07-30 — [PR #34374](https://github.com/zed-industries/zed/pull/34374). "windows: Fix message loop using too much CPU" merged 2025-08-12 — [PR #35969](https://github.com/zed-industries/zed/pull/35969)
- Linux history: "Linux port via Blade" merged 2024-02-07 — [PR #7343](https://github.com/zed-industries/zed/pull/7343). "gpui: Remove blade, reimplement linux renderer with wgpu" merged 2026-02-13 — [PR #46758](https://github.com/zed-industries/zed/pull/46758). Follow-up regressions were fixed in Feb 2026, for example X11 content scaling — [PR #49174](https://github.com/zed-industries/zed/pull/49174)
- Open wgpu/Linux issues as of Oct 2026:
  - Crash after GPU device-loss recovery on X11 — [#62998](https://github.com/zed-industries/zed/issues/62998)
  - Shadow struct alignment fails on some devices — [#65153](https://github.com/zed-industries/zed/issues/65153)
  - "gpui_wgpu maps unused Mesa software/GL stacks at instance init" — [#63327](https://github.com/zed-industries/zed/issues/63327)
  - Open PRs: "Only initialize the GL backend if Vulkan fails" [#65137](https://github.com/zed-industries/zed/pull/65137), "Don't crash when a GPU backend panics during init" [#62300](https://github.com/zed-industries/zed/pull/62300), "Allocate path render targets lazily" [#63289](https://github.com/zed-industries/zed/pull/63289)
- "Sluggishness in GNOME 50 / Wayland" (opened 2026-04-09, open, 42 comments). The UI freezes for up to 1.5 s, and the workaround is forcing XWayland — [#53522](https://github.com/zed-industries/zed/issues/53522)
- "Regression: blank window with GPU page faults on RDNA 4" (2026-03, closed) — [#50897](https://github.com/zed-industries/zed/issues/50897)
- Linux and FreeBSD are supported for both Wayland and X11 per GPUI Box docs — [docs.rs gpui-box](https://docs.rs/crate/gpui-box/0.1.1)
- There is also a web/wasm backend (`gpui_web`, PRs in Sep 2026) — [PR #63949](https://github.com/zed-industries/zed/pull/63949)

### Inferences
- For a macOS-first team, platform risk is low. On Linux, expect GPU-driver-specific bugs that you inherit and cannot fix easily. On Windows, DX11 is more than a year old and in production, but there are fewer independent reports about it.

### Gaps
- I found no systematic maturity comparison across platforms, and no crash-rate data per platform.

## Text input, IME (CJK/Korean), accessibility, text rendering, keyboard handling

### Takeaway
IME works in Zed, and Korean typing is broadly usable. But CJK IME bugs are still being filed and fixed every month in 2026, including in GPUI's own `input` example. Every custom text field in your app has to implement IME itself. Screen-reader support is new: AccessKit landed in GPUI on 2026-05-27, and coverage is still being built out. Treat accessibility as early-stage.

### Cited Findings
- Open IME issues in Oct 2026:
  - "IME candidate window is misplaced… (overlaps it in the terminal)" (2026-09-28) — [#64850](https://github.com/zed-industries/zed/issues/64850)
  - "Wayland: active IME preedit segment is not visually distinguished" (2026-10-08) — [#65302](https://github.com/zed-industries/zed/issues/65302)
  - "project_panel filename editing doesn't implement IME composition underline" (2026-10-10) — [#65453](https://github.com/zed-industries/zed/issues/65453)
  - "Pending multi-stroke keybindings (Ctrl+K) are intercepted by IME" — [#62764](https://github.com/zed-industries/zed/issues/62764)
  - "Chinese IME intermittently fails across editor, Agent panel, and Git commit input" — [#59882](https://github.com/zed-industries/zed/issues/59882)
  - Windows TSF GetTextExt rectangle bug — [#59636](https://github.com/zed-industries/zed/issues/59636)
- GPUI's own sample: "gpui: examples/input.rs misplaces the IME composition selection; copy can then panic" (2026-09-17) — [#64389](https://github.com/zed-industries/zed/issues/64389)
- Korean specific: "Hanja key in Korean IME doesn't fully work" (2026-01, open) — [#46319](https://github.com/zed-industries/zed/issues/46319). Also "Vim mode: jj mapping conflicts with Korean IME" (2025, closed) — [#38616](https://github.com/zed-industries/zed/issues/38616)
- Open fix PRs: "Fix CJK IME input with ASCII-capable sources and focus loss" — [#63405](https://github.com/zed-industries/zed/pull/63405); "IME eats next input after Ctrl+K in Wayland and Windows" — [#64597](https://github.com/zed-industries/zed/pull/64597)
- Accessibility:
  - "Windows: Screen reader accessibility missing completely" (opened 2025-10-24, still open) — [#41138](https://github.com/zed-industries/zed/issues/41138)
  - "gpui: Accesskit support" merged 2026-05-27 — [PR #56065](https://github.com/zed-industries/zed/pull/56065). Contributor status from #41138: tested with Windows Inspect and Linux Wayland (accerciser), not tested on X11, and macOS VoiceOver only partly tested. It is described as "somewhat a passion project". Follow-ups: exposing a11y identifiers (merged 2026-07-30, [PR #61926](https://github.com/zed-industries/zed/pull/61926)), and a macOS crash when a11y queries a closing window (closed 2026-07, [#61825](https://github.com/zed-industries/zed/issues/61825))
  - Longbridge's library claims "AccessKit roles, names, states, relationships, and actions are built into the interaction layer and covered by tests" — [GPUI Kit README](https://github.com/longbridge/gpui-component) (claim)
- Text rendering:
  - The monochrome glyph atlas uses bilinear filtering, causing "redundant blur… disproportionately affecting CJK" (2026-05, open) — [#57777](https://github.com/zed-industries/zed/issues/57777)
  - "Characters disappear and reappear when scrolling" (2026-07, open) — [#60303](https://github.com/zed-industries/zed/issues/60303)
  - Arabic-locale bracket shaping bug — [#65440](https://github.com/zed-industries/zed/issues/65440)
- Keyboard: "Tracking - Linux non-QWERTY keyboard support" (open since 2025-02) — [#25469](https://github.com/zed-industries/zed/issues/25469)

### Inferences
- For a Korean-speaking team, IME will mostly work through the editor/input components. Budget time for composition-underline and candidate-window-position bugs, especially in any custom text input you build, such as a ticket title or a chat box to agents.

### Gaps
- I did not test Korean 2-beolsik input personally. I found no dedicated Korean user report for gpui-component inputs.

## Ecosystem: gpui-component / GPUI Kit, apps built on GPUI, community size

### Takeaway
Longbridge's library is now branded "GPUI Kit" (repo longbridge/gpui-component). It is large and active, with 75+ components, and covers almost exactly what equipe needs: virtualized tables and lists, a tree-sitter code editor with LSP diagnostics, markdown, and dock layouts. Outside Zed and Longbridge Pro, the app ecosystem is small, mostly hobby or early projects.

### Cited Findings
- Repo stats via GitHub API on 2026-10-11: about 16.9k stars, last push 2026-10-11. Releases v0.6.2 (2026-09-18) → v0.7.0 (2026-09-28) → v0.7.1 (2026-10-05), so it releases about weekly — [GitHub longbridge/gpui-component](https://github.com/longbridge/gpui-component)
- Features it claims:
  - "75+ documented components"
  - Data tables with virtual scrolling, resizable columns and sorting
  - Virtual lists with variable item heights
  - A code editor "supporting 200K lines with Tree-sitter syntax highlighting and LSP diagnostics"
  - Markdown and HTML rendering
  - Dock layout with draggable tabs
  - WebAssembly, AccessKit, headless UI integration testing, and an optional JS extension runtime
  — [GPUI Kit README](https://github.com/longbridge/gpui-component) (vendor claims)
- Minimum hello-world binary for gpui-component is listed as about 12 MB, compared with Iced about 11 MB and egui about 5 MB (vendor claim, platform and profile unclear) — via search snippet of [crates.io gpui-component 0.5.0](https://crates.io/crates/gpui-component/0.5.0)
- Longbridge Pro, a commercial trading desktop app, is built on it — [GPUI Kit README](https://github.com/longbridge/gpui-component)
- Apps listed as shipping on GPUI (Oct 2026): Zed, Paneflow (83 stars; a terminal/agent multiplexer using alacritty_terminal), Waku, muxel, code-assistant, deck, zedis (about 2.1k stars), disktree (about 2.05k stars), Loungy (about 1.7k stars) — [baeseokjae blog, 2026-10-01](https://baeseokjae.github.io/posts/native-app-for-coding-agents-rust-gpui/)
- Terminal panes: the common route is `alacritty_terminal` v0.26.0 (2026-04-06), which Zed's own terminal also uses — [baeseokjae blog](https://baeseokjae.github.io/posts/native-app-for-coding-agents-rust-gpui/)
- Zed's blog featured Longbridge's Jason Lee (gpui-component author) as a "Community Champion" — [zed.dev blog](https://zed.dev/blog/community-champion-jason-lee)

### Inferences
- The closest prior art to equipe is Paneflow plus Zed's agent panel: native GPUI with alacritty_terminal panes and streaming logs. That makes the "agent board + logs + diffs + terminals" shape very plausible.
- Depending on gpui-component ties you to Longbridge's chosen GPUI snapshot and release pace. That is a second-party dependency with a single company behind it.

### Gaps
- I found no independent quality review of gpui-component's code editor or diff view. A diff view as a dedicated component is not confirmed. Zed has one, but it is not a reusable crate.

## Measured numbers: RAM, binary size, startup

### Takeaway
Measured: Zed itself idles at about 180–220 MB RSS on macOS, and its installers are 68–119 MB. These come from third-party measurements and are of varying quality. For a minimal GPUI app, the only figures I found are a stripped native module of about 17 MB on macOS arm64 and a vendor claim of about 12 MB. I found **no credible measured RAM figure for a GPUI hello-world**. That is the main gap, and it should be measured in-house.

### Cited Findings
- **Measured, Zed 1.0 (May 2026, M3 MBP 18 GB, macOS 15.4, median of 3 cold runs):**
  - Idle RAM with no agent: about 180 MB
  - Indexed with file tree open: about 220 MB
  - Agent panel active: about 340 MB
  - Cold start to editable cursor: 0.42 s
  - The RAM measurement method was not stated, and the process count was borrowed from another site
  — [Vantaige benchmark](https://vantaige.io/blog/zed-1-vs-cursor-vs-vscode-real-benchmark-2026)
- **Claim or secondary:** Zed 222 MB idle vs VS Code 3,549 MB, 5 processes vs about 23. An SEO-style comparison site with unclear method — [tech-insider.org](https://tech-insider.org/zed-vs-vscode-2026/)
- **Zed download sizes (v1.22.0, 2026-09-30):** macOS dmg 112 MB, Linux tar.gz 119 MB, Windows exe 68 MB — [baeseokjae blog](https://baeseokjae.github.io/posts/native-app-for-coding-agents-rust-gpui/). Zed bundles far more than GPUI: language servers glue, git, and so on.
- **Measured, minimal GPUI (gpuix project, 2026-09-06):**
  - On macOS arm64 release, the GPUI native `.node` module is 22 MB, and 17 MB after `strip -x`. The authors attribute it to "11 MB text section, unwind tables, Metal, font-kit, syntect", and say "almost all of the 33 MB is GPUI"
  - The Linux x86_64 hello world is 121 MB, but that includes a 78 MB Bun runtime, so the GPUI delta is about +43 MB, profile unspecified
  — [remorses/gpuix issue #58](https://github.com/remorses/gpuix/issues/58)
- **Claim:** gpui-component hello world about 12 MB minimum — [crates.io gpui-component 0.5.0](https://crates.io/crates/gpui-component/0.5.0)
- Real-world memory pathologies in Zed (application-level, not necessarily GPUI): "Zed using 100% of RAM on Windows" (2026-08, open) [#62133](https://github.com/zed-industries/zed/issues/62133); "High memory usage" (2026-09) [#64009](https://github.com/zed-industries/zed/issues/64009); a Windows crash after memory growth during heavy Agent use (2026-10-11) [#65465](https://github.com/zed-industries/zed/issues/65465)

### Inferences
- A GPUI app the size of equipe would plausibly land around 15–40 MB binary and, judging from Zed's 180 MB with far more features, well under 150 MB RSS. **This is an extrapolation, not a measurement.**

### Gaps
- No measured RSS or startup time for a GPUI hello world on any platform. Suggested in-house test: build `gpui` `examples/hello_world` in release, run it and check RSS (`ps -o rss`), `footprint` on macOS, and VRAM via Activity Monitor GPU memory or `nvidia-smi`. Do the same with a gpui-component Table plus Editor demo.

## Dev-velocity costs: compile times, learning curve, hot reload

### Takeaway
Expect a slow first build of 5–15 min, Rust's usual incremental rebuild cost, a learning curve around Entity/Context ownership, and no hot reload. This is the biggest practical cost for a small team that iterates on UI.

### Cited Findings
- "First build time: 5–15 minutes (GPUI + wgpu compile from source)" — [baeseokjae blog, 2026-10-01](https://baeseokjae.github.io/posts/native-app-for-coding-agents-rust-gpui/)
- Needs the latest stable Rust (README). Zed pins 1.95.0 and GPUI Box requires 1.97+ — [README](https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md); [baeseokjae](https://baeseokjae.github.io/posts/native-app-for-coding-agents-rust-gpui/); [gpui-box](https://docs.rs/crate/gpui-box/0.1.1)
- Programming model: state lives in `Entity`s "owned by GPUI and only accessible through an owned smart pointer similar to an Rc". Views are entities implementing `Render`, and styling is tailwind-like — [README](https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md)
- Idle-render pitfall in practice: Paneflow measured 6–8 idle repaints per second before moving to event-driven updates, and zero after. It coalesced PTY wakeups to 4 ms or 100 events — [baeseokjae blog](https://baeseokjae.github.io/posts/native-app-for-coding-agents-rust-gpui/)

### Inferences
- The app will feel lightweight only if the team learns GPUI's notify/observe model. Naive streaming of several agent logs can cause constant repaints.

### Gaps
- I found no hot-reload mechanism for GPUI UI code, and Zed issue searches turned up only unrelated "hot reload" topics. Absence is likely but not proven. Third-party crates like `subsecond` or `hot-lib-reloader` might work, but I found no GPUI-specific success report.
- I found no quantitative incremental-rebuild times for a mid-size GPUI app.

## Known pitfalls: VRAM, idle CPU, battery, integrated GPUs

### Takeaway
GPUI's upstream frame renderer redraws the whole window, with no retained/incremental rendering upstream. CPU per frame is a few milliseconds even for an unchanged window. Longbridge built GPUI Fast precisely to fix this. There are documented idle-CPU regressions in Zed and GPU driver issues, mainly on Linux. I found no hard VRAM numbers.

### Cited Findings
- GPUI Fast's recorded Linux release-build CPU time per frame:
  - Unchanged window: 3.31 ms baseline vs 0.19 ms
  - Sidebar scrolling: 5.95 vs 0.79 ms
  - Streaming quotes: 5.06 vs 1.35 ms
  - The baseline is upstream `gpui-pre` 0.3.7
  — [longbridge/gpui-fast README](https://github.com/longbridge/gpui-fast) (vendor-measured)
- "GPUI overdraws too much" (opened 2024-02, still open, 19 comments) — [#8043](https://github.com/zed-industries/zed/issues/8043)
- Idle CPU incidents: "New update is using up to 400% of cpu (260% just idle)" (2025-10, closed) — [#42032](https://github.com/zed-industries/zed/issues/42032); "Windows: Fix message loop using too much CPU" (2025-08) — [PR #35969](https://github.com/zed-industries/zed/pull/35969); "Linux: cpal_alsa_out pins one CPU thread at 100%" (2026-09, closed; audio, not GPUI) — [#64962](https://github.com/zed-industries/zed/issues/64962)
- Integrated-GPU and driver issues: "Windows: Zed window briefly turns black (Intel Graphics)" (2025, closed) — [#33191](https://github.com/zed-industries/zed/issues/33191). The wgpu init loads unused Mesa GL stacks, which raises baseline memory — [#63327](https://github.com/zed-industries/zed/issues/63327). Path render targets are not allocated lazily yet (open PR) — [#63289](https://github.com/zed-industries/zed/pull/63289)
- Apple-silicon unified-memory optimization was reverted (2025-12-16) — [PR #45022](https://github.com/zed-industries/zed/pull/45022)

### Inferences
- With event-driven updates and static content, GPUI idles near zero CPU because it only renders on notify. During multi-log streaming, though, each update repaints the whole window. On battery and integrated GPUs this matters more than with a retained-mode toolkit. Throttling or coalescing log updates is mandatory.

### Gaps
- I found no VRAM measurements for Zed or a GPUI app, and no battery-drain benchmarks against Electron, Tauri or native toolkits.

## Overall assessment (synthesis for the report writer)

### Takeaway
GPUI is not "overkill" for equipe's workload. A board, streaming logs, code and diff views, and terminals are exactly what GPUI and gpui-component were built for, and Zed shows that about 200 MB RSS is reachable for a full IDE. Small binaries in the roughly 12–20 MB range also look plausible. The risks are organizational rather than technical:
- no maintained crates.io release from Zed
- a split fork ecosystem
- pre-1.0 churn
- slow Rust builds and no hot reload
- IME and accessibility still maturing
- Linux GPU-driver fragility

It is a good fit if the team is comfortable in Rust and accepts pinning to gpui-component's GPUI snapshot. It is risky if UI iteration speed or screen-reader support is a hard requirement.

### Cited Findings
- See sections above. Key anchors: [crates.io gpui](https://crates.io/api/v1/crates/gpui), [discussion #1856](https://github.com/longbridge/gpui-component/discussions/1856), [Vantaige benchmark](https://vantaige.io/blog/zed-1-vs-cursor-vs-vscode-real-benchmark-2026), [gpuix #58](https://github.com/remorses/gpuix/issues/58), [GPUI Kit](https://github.com/longbridge/gpui-component), [AccessKit PR #56065](https://github.com/zed-industries/zed/pull/56065)

### Inferences
- Recommended de-risking spike (1–2 days): build a gpui-component app with one Table, three streaming log panes, one Editor with a diff, and one alacritty_terminal pane. Measure RSS, VRAM, idle CPU, the stripped binary, and Korean IME behavior on macOS, plus Linux if needed.

### Gaps
- No first-hand measurements. The Rust toolchain was unavailable in the research environment.
