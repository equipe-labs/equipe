# Third-party notices

equipe is MIT-licensed (see [LICENSE](./LICENSE)). It works with the
third-party software below, under that software's own license. Rust crates
equipe builds against are listed with their licenses in `Cargo.lock` and on
crates.io; the patched GPUI crates under `vendor/` keep their own license files.

## agent-desktop

- **What:** the accessibility engine behind the members' desktop tools
  (`snapshot`, `find`, `act`, …): it reads desktop apps' accessibility trees and
  acts on them. See [docs/DESKTOP-CONTROL.md](./docs/DESKTOP-CONTROL.md).
- **Project:** <https://github.com/vercel-labs/agent-desktop> (Vercel Labs)
- **Version:** 1.2.0, pinned in `crates/desktop-control/src/engine/pin.rs`
- **License:** Apache License 2.0 — full text in
  [licenses/agent-desktop-Apache-2.0.txt](./licenses/agent-desktop-Apache-2.0.txt),
  identical to the `LICENSE` file of the upstream repository at tag `v1.2.0`.
  The upstream project publishes no `NOTICE` file at that version.
- **How equipe uses it:** equipe neither bundles nor modifies agent-desktop, and
  links none of its code. equipe's daemon (or `equipe desktop setup`) downloads the unmodified
  upstream release archive for the machine from GitHub Releases, checks its
  SHA-256 against the value pinned in equipe (the release's `checksums.txt`;
  the archives also carry a Sigstore build attestation), checks each unpacked
  file's SHA-256, and installs it under `~/.equipe/desktop/agent-desktop/1.2.0/`
  together with a copy of the license above. The equipe desktop helper runs the
  `agent-desktop` executable as a separate process.
- The test fixtures under `crates/desktop-control/src/engine/fixtures/` are
  written by hand in the JSON shape agent-desktop documents; they contain no
  agent-desktop code.

## UI design skills — adapted guidance

The `ui-*` skills under `crates/core/skills/` are equipe's own writing. They
adapt rules and structure, rewritten in our own words, from two MIT-licensed
projects; the full list of sources they draw on is in
`crates/core/skills/ui-review/references/sources.md`.

- **Web Interface Guidelines** — <https://github.com/vercel-labs/web-interface-guidelines>
  (Vercel Labs). License: MIT, Copyright (c) 2025 Vercel Labs — full text in
  [licenses/web-interface-guidelines-MIT.txt](./licenses/web-interface-guidelines-MIT.txt).
  Its interaction, form, motion, layout and content guidance informs
  `ui-polish`, `ui-forms`, `ui-motion`, `ui-accessibility`, `ui-foundations`
  and `ui-writing`.
- **Mobbin skills** — <https://github.com/mobbin/skills> (Mobbin). License:
  MIT, Copyright (c) 2026 Mobbin — full text in
  [licenses/mobbin-skills-MIT.txt](./licenses/mobbin-skills-MIT.txt). The
  `redesign-screen` skill's job walk, its cap on changes per pass and its
  rules for placing a new part are adapted in `ui-review` and
  `ui-layout-and-states`.
