# What a judge can try, and what it needs

equipe orchestrates AI coding agents. They can run on **the user's own** Claude Code or Codex
(ChatGPT) plan — or, with only a Token Factory key and the free, open-source Codex CLI, on
**NVIDIA Nemotron through Nebius Token Factory**, no subscription at all. The Nemotron review runs
on Token Factory too. This table says exactly what each part needs, so nobody is surprised.

Legend: **TF key** = a Nebius Token Factory API key in `NEBIUS_API_KEY` (a judge's own, or one
the owner provides — see [judge-access.md](judge-access.md); not decided yet).
**Engine** = Claude Code signed in with a Claude plan, or Codex with a ChatGPT plan.
**codex CLI** = OpenAI's open-source Codex CLI installed (`npm install -g @openai/codex` or
`brew install codex`), free, **not** signed in — equipe uses it as the agent loop only.

| What | Needs an engine (Claude/Codex)? | Needs a TF key? | Calls NVIDIA Nemotron on Nebius? | Notes |
|---|---|---|---|---|
| `equipe review --models` | No | **Yes** | No (lists models: `GET /v1/models`) | Shows the account's NVIDIA Nemotron models and the one `equipe review` would use. |
| `equipe review` / `equipe review --base <ref>` | No | **Yes** | **Yes** (`POST /v1/chat/completions`) | The fastest real path. Any git repository with changes; macOS or Linux CLI from the install script, a release, or `cargo install --path crates/cli`. |
| **The whole squad on Nemotron**: `equipe`, `equipe try`, the desktop app — members writing code, running tests, handing in | **No** — the codex CLI instead (free, no sign-in) | **Yes**, in the daemon's environment (`equipe daemon stop; equipe daemon run` from the shell with the key) | **Yes** (`POST /v1/responses`, many requests per turn) | **Mock-verified only** (the real codex CLI against a stand-in Responses API; `verification.md` §3.1) — not yet run against Token Factory. Agent turns can use many tokens. With no engine signed in, members start on Nemotron by themselves. |
| Automatic review of handed-in work (daemon) | No, with the squad on Nemotron (above); else Claude Code or Codex | **Yes**, in the daemon's environment | **Yes** | Shown in the desktop app's ticket peek and on the ticket's record. |
| Desktop app showing a review (UI only) with the fake daemon: `FAKE_REVIEW=1 cargo run -p equipe-desktop --example fake_daemon` + `cargo run -p equipe-desktop` | No | No | **No** — canned reviews labelled `nvidia/fake-nemotron (FAKE_REVIEW)` | Builds from source on macOS. Shows the screen only; it is **not** evidence of a Nebius call. |
| `cargo test --workspace` | No | No | **No** — a local mock server | Proves the client's behaviour (retries, errors, parsing, redaction) against canned answers, and — where the codex CLI is installed — runs the real codex against a stand-in Responses API; not the real API. |
| `equipe status`, `equipe roles`, `equipe lang`, `equipe --help` | No | No | No | Basic CLI. |
| `equipe` (the squad console), `equipe try` | **Yes** — or the TF key + codex CLI (row above) | Optional with an engine (reviews only) | Only via Nemotron members or the automatic review | First run says "No engine is signed in yet" with neither an engine nor a key (+ codex CLI). |
| Phone app via `equipe demo serve` | No | No | No | A scripted demo squad for store reviewers; no model runs. |

## The honest summary

- With **only a TF key**, a judge can run the real NVIDIA-on-Nebius feature end to end with
  `equipe review` on any repository.
- The **full squad** (agents writing code, then Nemotron reviewing the hand-in) now works with
  **only a TF key + the free codex CLI** — no Claude or ChatGPT plan. This is **mock-verified**
  (the real codex CLI driven by equipe against a stand-in Token Factory), not yet run against
  the real one: say "mock-verified" until `verification.md` §3.2 is filled in. How well a given
  Nemotron model drives the agent's tools is untested.
- Whether "bring your own key / subscription" satisfies the rule that judges can test
  **free and without restriction until 2026-12-23 12:00 PT** is **not confirmed**. Options are in
  [judge-access.md](judge-access.md); the decision is the owner's, and may need the organizers'
  written confirmation.
