# What a judge can try, and what it needs

equipe orchestrates AI coding agents that run on **the user's own** Claude Code or Codex
(ChatGPT) plan. The Nemotron review is the part that runs on **Nebius Token Factory**. This
table says exactly what each part needs, so nobody is surprised.

Legend: **TF key** = a Nebius Token Factory API key in `NEBIUS_API_KEY` (a judge's own, or one
the owner provides — see [judge-access.md](judge-access.md); not decided yet).
**Engine** = Claude Code signed in with a Claude plan, or Codex with a ChatGPT plan.

| What | Needs an engine (Claude/Codex)? | Needs a TF key? | Calls NVIDIA Nemotron on Nebius? | Notes |
|---|---|---|---|---|
| `equipe review --models` | No | **Yes** | No (lists models: `GET /v1/models`) | Shows the account's NVIDIA Nemotron models and the one `equipe review` would use. |
| `equipe review` / `equipe review --base <ref>` | No | **Yes** | **Yes** (`POST /v1/chat/completions`) | The fastest real path. Any git repository with changes; macOS or Linux CLI from the install script, a release, or `cargo install --path crates/cli`. |
| Automatic review of handed-in work (daemon) | **Yes** — a squad member must do the work and hand it in, and members run on Claude Code or Codex | **Yes**, in the daemon's environment | **Yes** | Shown in the desktop app's ticket peek and on the ticket's record. Without an engine there is no hand-in, so no automatic review. |
| Desktop app showing a review (UI only) with the fake daemon: `FAKE_REVIEW=1 cargo run -p equipe-desktop --example fake_daemon` + `cargo run -p equipe-desktop` | No | No | **No** — canned reviews labelled `nvidia/fake-nemotron (FAKE_REVIEW)` | Builds from source on macOS. Shows the screen only; it is **not** evidence of a Nebius call. |
| `cargo test --workspace` | No | No | **No** — a local mock server | Proves the client's behaviour (retries, errors, parsing, redaction) against canned answers; not the real API. |
| `equipe status`, `equipe roles`, `equipe lang`, `equipe --help` | No | No | No | Basic CLI. |
| `equipe` (the squad console), `equipe try` | **Yes** | Optional (reviews only) | Only via the daemon's automatic review | First run says "No engine is signed in yet" without one. |
| Phone app via `equipe demo serve` | No | No | No | A scripted demo squad for store reviewers; no model runs. |

## The honest summary

- With **only a TF key**, a judge can run the real NVIDIA-on-Nebius feature end to end with
  `equipe review` on any repository.
- The **full squad** (agents writing code, then Nemotron reviewing the hand-in) additionally
  needs a Claude or ChatGPT plan for the agents. equipe never pays for or pools anyone's
  engine usage, so it cannot provide one.
- Whether "bring your own key / subscription" satisfies the rule that judges can test
  **free and without restriction until 2026-12-23 12:00 PT** is **not confirmed**. Options are in
  [judge-access.md](judge-access.md); the decision is the owner's, and may need the organizers'
  written confirmation.
