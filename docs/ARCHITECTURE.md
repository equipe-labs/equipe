# Equipe architecture

Design decisions and the shape of the code. This is a living document; the
[roadmap](./ROADMAP.md) says what is built and what is next.

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
   pause at an approval gate.
5. **Sign in once.** However many engines a squad uses, a person signs in to each
   of their plans once. When an engine wants its own sign-in, equipe satisfies it
   from the login it already holds where that is allowed, and asks the person only
   when it isn't. Adding an engine must never add a login prompt to the daily path.
6. **Honest identity, and only the credentials we are entitled to.** equipe names
   itself to every backend and never impersonates another client. It holds a
   credential only when the provider allows a third-party tool to: the ChatGPT
   login, yes; Claude.ai credentials, never — Claude Code keeps those.

## Code layout

An all-Rust workspace. The domain lives in one core that every front end shares.

| Crate | Owns | Depends on |
|---|---|---|
| `equipe-protocol` (`crates/protocol`) | The contract between the brain and every screen: `Command`s in, `Event`s out | — |
| `equipe-core` (`crates/core`) | The brain: the `runtime` that runs sessions on engines; roles; the board and agent logs in SQLite; worktree sandboxes | protocol, providers |
| `equipe-providers` (`crates/providers`) | Sign-in and engines: Codex (OAuth, models, Responses turns), the user's Claude Code | — |
| `equipe` (`crates/cli`) | Screens: the command line and the terminal UI | protocol, core |
| `equipe-desktop` (`crates/desktop`) | The desktop app on GPUI Kit — see [DESKTOP.md](./DESKTOP.md) | core |

### The brain and the screens

The core speaks in messages. A screen sends `Command`s — start a session,
submit a message, interrupt, close — and renders the `Event`s that come back:
text as it streams, tool use, turn ends, branches landed, errors with the
command that fixes them. Screens never call an engine and hold no state beyond
what they show. Codex's `core` / `tui` / `exec` / `app-server` split is the model.

- Inside one process, messages are Rust values on a channel. Every message also
  serializes to JSON, so `equipe serve` can later carry the same contract over a
  socket as JSON-RPC 2.0 — the shape of ACP, MCP and LSP — and the terminal,
  the desktop app, Slack or Zed attach as clients.
- The runtime owns each session: its engine and model, the conversation so far
  (Codex resends history; Claude Code resumes with `--resume`), and an editing
  session's worktree. One turn runs at a time per session; Esc interrupts it at
  once.
- Because both sides meet only at the protocol, each is tested alone: the
  runtime by sending commands against a stand-in engine, the screens by feeding
  events and checking what they draw.

## Layers

| Layer        | Equipe concept | Notes |
| ------------ | -------------- | ----- |
| Provider     | `equipe-providers` | how we sign in and which backends we reach |
| Model        | model id / alias | Codex: chosen from the account's live catalog; Claude Code: its own aliases |
| Engine       | `llm` \| `cli` \| `acp` | the thing that runs a turn — see below |
| Role         | `equipe_core::roles` | id + mandate today; charter, toolset, engine next |
| Orchestrator | director loop | plans, assigns, gates (Phase 2) |
| Board        | `equipe_core::store` | tickets, events, agent logs in SQLite |

## Engines

| Kind | What runs a turn | Today |
|---|---|---|
| `llm` | A thin provider call — no tools, no loop | **Codex Responses** (`openai::responses`): one streamed turn on the ChatGPT backend. For non-coding roles (PO, Designer) |
| `cli` | A local coding-agent CLI, driven headless | **Claude Code** (`claude_code`): `claude -p --output-format stream-json`, reporting text and tool use as it works |
| `acp` | An external coding agent over the Agent Client Protocol | **Planned** for Codex edits via `codex-acp`; later any ACP agent |

Engine access is explicit. A Claude Code turn is `ReadOnly` (`--permission-mode
dontAsk`) unless the caller asks for `EditFiles` (`acceptEdits`): file writes inside
the working directory only, with shell commands and outside paths still needing an
approval that a headless run cannot give.

**ACP: version 1.** v2 is a draft and removes the client-provided `fs/*` and
`terminal/*` methods; v1's delegation of file access to the client is what lets
equipe confine an agent to its worktree. equipe is an ACP *client* toward engines;
exposing equipe itself as an ACP *agent* (so Zed or JetBrains can drive a squad) is
a later option.

## Sign-in

### Codex: equipe signs in

The mechanism OpenAI's own open-source Codex CLI uses:

- Public Codex OAuth client + PKCE (S256) against `auth.openai.com`, with a state
  value against CSRF. The loopback callback listens on **both** `127.0.0.1` and
  `::1` — the redirect URI says `localhost`, which browsers may resolve to IPv6
  first. A browser counts as opened only when the opener exits successfully;
  otherwise the link is printed.
- Traffic to `chatgpt.com/backend-api/codex` with **honest identity headers**
  (`originator: equipe`, `User-Agent: equipe/<version>`), plus the workspace headers
  the backend derives from the token: `ChatGPT-Account-ID` and, for
  residency-enforced workspaces, `x-openai-internal-codex-residency`.
- The model catalog is requested at a named Codex client level
  (`CATALOG_CLIENT_VERSION`): the backend hides models above the `client_version`
  it is sent and answers an empty list with HTTP 200. Hidden models are dropped and
  the rest ranked by priority.
- Credentials live in `~/.equipe/credentials/openai.json` (0600 in a 0700
  directory, replaced atomically), refreshed 60 s before expiry.

When Codex edits arrive over `codex-acp`, the adapter would ask for its own login.
Under principle 5, equipe will give it a private `CODEX_HOME` seeded from the login
it holds, and fall back to the adapter's own sign-in only if that fails.

### Claude Code: bring your own

Anthropic's terms
([Claude Code legal and compliance](https://code.claude.com/docs/en/legal-and-compliance))
don't let third-party developers offer Claude.ai login, route requests through
Free/Pro/Max credentials on their users' behalf, or collect, store or intermediate
Claude.ai credentials. They do allow an end user to sign in to the **unmodified
Claude Code binary** with their own plan, including where another product runs it,
provided that product doesn't remove or restrict its sign-in methods and each user
is billed under their own agreement. So equipe:

- runs the user's installed `claude` and never ships, patches or wraps a copy;
- never reads, stores or forwards Claude credentials, and never injects a token —
  the private-home trick planned for Codex is **not** used here;
- asks `claude auth status --json` for status (non-secret fields only) and starts
  `claude auth login` when the user chooses to sign in;
- passes the environment through unchanged rather than stripping auth variables,
  which would restrict the user's own choice of method — and instead shows in
  `equipe status` whether usage is a subscription or per-token billing;
- never passes `--bare`, which skips the subscription login.

Open questions: whether a distributed orchestrator driving `claude -p` on a
subscription sits fully inside the "unmodified binary" allowance is not spelled
out — confirm with Anthropic before advertising subscription use. Claude Code plans
to make `--bare` the default for `-p`; equipe will then need an explicit opt-out.

## Board

SQLite through `rusqlite` (`equipe_core::store`). Today it holds tickets and
per-agent log lines; logs never live in UI memory — writers append in batched
transactions and a reader asks only for the rows it will draw.

Next: `events`, an immutable log of created / assigned / note / handoff / status /
approval / artifact. Status flow: `backlog → todo → in_progress → in_review → done`
(plus `blocked`, `cancelled`); `approvalGates` lists statuses that pause for a human.

### Plans are notes; tickets are commitments

Engines report their own step-by-step plan (ACP `agent-plan`). It is **not** a
ticket, just as a developer's notebook is not the team's tracker:

1. A plan belongs to the ticket being worked and is shown beside it as progress
   (`2/4`). Each update replaces the previous plan whole (ACP requires this).
2. Plan entries never become tickets on their own.
3. Work outside the ticket's scope is **proposed** as a new ticket (through a board
   tool equipe offers the agent over MCP); the PO or a person accepts it.
4. A plan with every step done does not close the ticket. Review and the approval
   gate do.

## Execution: git worktrees

A code-writing agent never works in the user's checkout. `equipe try --edit` — and
the orchestrator per ticket in Phase 2 — cuts a branch from `HEAD`
(`equipe/try-<id>`), checks it out in a worktree under
`~/.equipe/worktrees/<repo>/<id>` (outside the repo, so `git status` stays clean),
runs the engine there, commits what changed to the branch and removes the worktree.
What remains is one reviewable branch. A run that changes nothing leaves no branch;
a run that fails keeps its partial changes on it.

## Terminal

The command line is a product surface, styled per [BRAND.md](./BRAND.md):

- Monochrome; one cool accent, used only for success (✓) and edits (✎). Weight and
  dimming carry the hierarchy. No emoji.
- Text wraps to the terminal width between words, keeping indents, counting
  Hangul/CJK as two columns; a word longer than a line (a URL) stays whole.
- Answers go to stdout, everything else to stderr, so `equipe try … > file` keeps
  only the answer. Plain text when piped or under `NO_COLOR`.
- Every failure ends with the command that fixes it (`→ equipe login codex`).
- Interactive choices are menus at a terminal and sensible defaults in scripts.

### The team console

`equipe` with no command opens the terminal UI (`ratatui`). It takes the whole
screen (alternate screen) because a team means several agents updating at once —
something a scrolling transcript can't show — and prints a summary of what
happened (branches landed, tokens) to the normal screen when it closes, so the
record stays in the terminal's history.

- Left: the team — each agent, its state (`● working`, `○ ready`), its model or
  branch. Right: the selected agent's conversation and work. Bottom: the message.
- Pastes arrive whole (bracketed paste): a newline inside a paste never sends.
  A multi-line draft shows its size before it's sent.
- Enter sends · Alt-Enter / Ctrl-J add a line · Esc stops the agent · Tab
  switches agents · Ctrl-C clears the draft, twice quits · `/new`, `/edit`,
  `/close`, `/help`, `/quit`.
- Drawn only when something changed; nothing redraws on a timer.

Next: the board fills the team pane, with approvals waiting in a fixed place.

## Desktop

`equipe-desktop` renders the same core in a native GPUI window. Its rules
("draw nothing when nothing changed", logs never in UI memory) and measurements are
in [DESKTOP.md](./DESKTOP.md); the stack research behind it is in
[research/desktop-stack](./research/desktop-stack/README.md).
