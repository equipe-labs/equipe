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

## VNC (the desktop app's Screen panel)

- **What:** the Screen panel shows, and lets the person drive, the desktop of the
  machine a remote daemon runs on, through a VNC server on that machine. See
  [docs/DESKTOP.md](./docs/DESKTOP.md), "The Screen panel".
- **The client is equipe's own** (`crates/rfb`, MIT): written from RFC 6143 and
  the community RFB extensions (Tight, ZRLE, continuous updates, fences); no VNC
  project's code is copied or linked. It builds on these crates from crates.io,
  each listed in `Cargo.lock`: `des` 0.8 (RustCrypto, MIT OR Apache-2.0) for VNC
  authentication, `flate2` (MIT OR Apache-2.0) for zlib, `image` (MIT OR
  Apache-2.0) for JPEG.
- **VNC servers are not bundled.** The person installs one on the far machine
  if it has none — TigerVNC (`x0vncserver`, GPL-2.0), x11vnc (GPL-2.0), GNOME
  Remote Desktop (GPL-2.0), or macOS Screen Sharing — and equipe talks to it over
  the RFB protocol as a separate program.
