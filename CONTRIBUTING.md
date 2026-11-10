# Contributing to equipe

Thanks for helping. equipe is open source under the [MIT licence](LICENSE), and issues and
pull requests are welcome from anyone.

By taking part you agree to follow our [Code of Conduct](CODE_OF_CONDUCT.md). Security issues
go through [SECURITY.md](SECURITY.md), never a public issue.

## Before you start

- **Small fixes** (a typo, a clear bug with an obvious fix, a missing translation): open a pull
  request directly.
- **Anything larger** (a new feature, a change to how members behave, a new dependency, a
  change to the protocol between the daemon and its screens): open an issue first so we can
  agree on the approach before you spend time on it.
- Search [existing issues](https://github.com/equipe-labs/equipe/issues) before opening a new
  one.

## Setting up

You need git and a stable [Rust](https://rustup.rs) toolchain. The desktop app builds on macOS
only; everything else builds on macOS and Linux.

```bash
git clone https://github.com/equipe-labs/equipe && cd equipe
cargo test --workspace --exclude equipe-desktop   # what CI runs; no network or sign-in needed
cargo run -p equipe -- status                     # the CLI from source
cargo run --release -p equipe-desktop             # the desktop app (macOS, see docs/DESKTOP.md)
```

[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) explains the shape of the code. The README's
Contributing section lists what each crate does.

## Making a change

- **Keep the diff focused.** One change per pull request. Don't reformat or rename code you
  aren't otherwise changing; the codebase is not run through `cargo fmt` as a whole.
- **Write the test first** when you fix a bug: a test that fails without your fix and passes
  with it. New behaviour comes with tests.
- **Before you push**, run what CI runs:

  ```bash
  cargo test --workspace --exclude equipe-desktop --locked
  cargo clippy --workspace --exclude equipe-desktop --all-targets --locked -- -D warnings
  ```

  If you changed the desktop app, run `cargo test -p equipe-desktop` on macOS too.
- **Desktop screenshots** for a pull request can be drawn offscreen against the example daemon
  (`EQUIPE_SNAPSHOT_HIDDEN=1`, [`scripts/design-shot.sh`](scripts/design-shot.sh)), so no real
  squad or screen is captured.

## Words people read

equipe is used in eight languages: English, German, Spanish, French, Japanese, Korean, and
Simplified and Traditional Chinese.

- Every string a person sees goes in the locale files (`crates/*/locales/*.json`), in all eight
  languages. A test fails if a key is missing from one of them.
- Code, comments and docs are written in English. Non-English text belongs only in locale files
  and tests; a test checks this too.
- Nothing may assume one country, language or market by default.

## Changes to how members behave

Members' instructions (their charters, tool descriptions and skills) decide how well the squad
works, so changes to them are held to a higher bar:

- A pinned-prompt test fails when a member's instructions change. Update the pin only on
  purpose, and say in the pull request what changed and why.
- Behaviour changes are checked with the quality gate, which compares the squad before and
  after on real tasks: see [docs/quality-gate.md](docs/quality-gate.md). Include the results in
  your pull request.

## Safety rules that don't bend

These protect the people who run equipe. Pull requests that weaken them won't be merged:

- Approvals, acceptances, merges and settings changes are the person's. A member's own process
  must never be able to send them (`person_only` in `crates/core/src/permissions.rs`).
- equipe never reads, stores or forwards Claude credentials.
- Nothing leaves the person's machine unless they turned a feature on; telemetry is opt-in and
  never carries content, names or paths ([docs/TELEMETRY.md](docs/TELEMETRY.md)).

## Pull requests

- Describe what changed and why, and how you tested it. Link the issue it closes.
- Keep commits readable; we merge with merge commits, so your history stays as you wrote it.
- A maintainer will review. We may ask for changes; that's normal.

By contributing, you agree that your contribution is licensed under the project's MIT licence.
