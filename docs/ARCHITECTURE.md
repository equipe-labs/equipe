# Equipe architecture

Design decisions and the shape of the code. This is a living document.

## Principles

1. **Don't rebuild the agent loop.** openclaw and hermes-agent show that a robust
   single-agent loop (compaction, retries, provider adapters, safety) is a large,
   mature effort. Equipe's value is the **team orchestration layer**, so we drive
   existing engines instead of reimplementing one.
2. **The board is the single source of truth.** Agents never coordinate by
   free-form conversation; they read/write tickets and append immutable events.
   This keeps handoffs auditable and avoids N² chatter.
3. **Roles are data, not code.** A role = charter (prompt) + toolset + permissions
   + preferred engine. Adding a role is a config change, not a new subsystem.
4. **Humans own the gates.** Irreversible or outward-facing steps (merge, deploy)
   pause at an approval gate. Borrowed from openclaw's admission/approval and
   hermes' `_run_conversation_with_approval`.

## Layers (mirrors openclaw's clean separation)

| Layer        | Equipe concept        | Notes                                                      |
| ------------ | --------------------- | ---------------------------------------------------------- |
| Provider     | `ProviderId` + auth   | how we authenticate & pick models (OpenAI OAuth first)     |
| Model        | `EngineRef.model`     | e.g. `openai/gpt-5.6`                                       |
| Engine       | `EngineKind`          | `acp` \| `cli` \| `llm` — the thing that runs a turn        |
| Role         | `RoleSpec`            | charter + toolset + permissions                            |
| Orchestrator | director loop         | plans, assigns, gates                                      |
| Board        | SQLite + git          | tickets, events, artifacts                                 |

## Engines: ACP-first hybrid

- **`acp`** — drive an external coding agent over the Agent Client Protocol.
  This is the primary path: one uniform interface to Codex, Claude Code, Gemini,
  opencode, etc. (openclaw already binds all of these over ACP; hermes exposes
  itself *as* an ACP server — so interop already exists in the ecosystem.)
- **`cli`** — spawn a local coding-agent CLI process and parse its I/O. Fallback
  for engines without a stable ACP surface.
- **`llm`** — a thin provider call for non-coding roles (PO/Designer) that don't
  need a full coding harness.

The ChatGPT-subscription setup: `EngineRef { kind: "acp"|"cli", backend: "codex",
provider: "openai", model: "openai/gpt-5.6" }` authenticated via Codex OAuth.

## Auth: OpenAI OAuth subscription (headline path)

Same mechanism OpenAI's own open-source Codex CLI uses:

- Public Codex OAuth client + PKCE against `auth.openai.com` (loopback callback or
  device code), yielding the user's own ChatGPT access/refresh tokens.
- Model traffic to `chatgpt.com/backend-api/codex`, with **honest third-party
  identity headers** (originator/version) — we do not impersonate the official CLI.
- Tokens stored locally under `~/.equipe/credentials/`, refreshed on expiry.
- `AuthMethod` also covers `api-key` and `none`; providers are pluggable so a squad
  can mix subscription + API key + local models per role.

## Board

SQLite (via `better-sqlite3`) under `.equipe/` in the project. Two core tables:

- `tickets` — id, title, body, status, role, assignee, dependsOn, branch, timestamps.
- `events` — immutable log of created/assigned/note/handoff/status/approval/artifact.

Status flow: `backlog → todo → in_progress → in_review → done` (plus `blocked`,
`cancelled`). `approvalGates` lists statuses that pause for a human.

## Execution: git worktrees

Each code-writing agent works in its own `git worktree` off the project repo, so
parallel work is isolated and every change lands as a reviewable branch/PR. Non-code
roles produce artifacts (specs, designs) attached to tickets as events.

## Roadmap (MVP slices)

1. **Board + roles + CLI** (`init`, `roles`, `board`). ← current
2. **Codex OAuth `login`** (headline auth path).
3. **One ACP engine adapter** + a single role (backend) completing one ticket in a worktree.
4. **Orchestrator loop**: PO ticket → assign → engineer → QA → approval gate.
5. Multi-role, multi-engine, per-role provider mixing.
