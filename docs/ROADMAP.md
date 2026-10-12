# Equipe roadmap

How we build equipe. This is the canonical plan; `docs/ARCHITECTURE.md` covers the design.

## Five principles

1. **Walking skeleton first.** Build one thin path that pierces every layer, then
   thicken it. Don't widen before the spine works end to end.
2. **De-risk the scary part first.** The real unknown isn't the board or the CLI —
   it's *"a coding agent, running on the user's own plan, edits real files."* If
   that doesn't hold, the premise doesn't. Prove it before plumbing.
3. **Don't reinvent the loop.** The agent loop is not our product. We keep mounting
   on mature engines (Codex, Claude Code) and own only the team layer.
4. **Dogfood from day one.** Build equipe with equipe as soon as it can hold a tool.
   It's both a forcing function and the best story.
5. **Every phase ships a demo.** This OSS has to spread, so the demo *is* the
   marketing. Each phase ends with one gif/video of something real.

## Phases

Each phase names the magic moment it unlocks, the risk it retires, and its
definition of done. Ship a demo at the end of each.

### Phase 0 — Foundations ✅
Repo, CLI skeleton, domain types, and the headline auth path.

### Phase 1 — Prove the pipe ✅ (Codex edits pending)
One agent, running on the user's own plan, does real work. No board yet.
- **1a — token reaches the model.** ✅ `equipe status` refreshes a stale token,
  reaches the Codex backend and lists the account's models.
- **1b-i — the model answers.** ✅ `equipe try "<prompt>"` streams a turn on
  **Codex** (ChatGPT plan, signed in by equipe) and on **Claude Code** (the user's
  own install and plan).
- **1b-ii — an agent edits a repo.** ✅ on Claude Code: `equipe try --edit` cuts a
  branch, works in an isolated worktree and leaves a reviewable branch.
  ⬜ on Codex: through an ACP engine (`codex-acp`), with the login-once handoff.
- **Retired:** subscription-driven execution works end to end, on both engines,
  verified on real accounts. Found and fixed on the way: the residency header, a
  browser that "opened" without opening, an IPv6-only callback failure, and a model
  catalog that came back empty for a low client version.
- **Demo:** "logged in with my plan, an agent edited my repo — on its own branch."

### Phase 2 — The spine ← next
Board + one role + one ticket → a real branch/diff.
- ✅ **Core speaks in events** (`equipe-protocol` + the core runtime), so the CLI,
  the terminal UI and the desktop app render one stream.
- SQLite board in `equipe-core` (`tickets`, `events`) + `equipe board`.
- One `backend` role (charter + toolset) + an orchestrator that takes a ticket,
  runs the engine in a worktree (the `--edit` sandbox, per ticket) and lands a
  branch. Human reviews.
- ✅ **Interactive `equipe` v0**: the team console in the terminal — several agents
  at once, streaming, conversation memory, bracketed paste, `/new` `/edit` `/close`.
  Next: approvals inline, the board in the team pane.
- **Retires:** orchestrator ↔ board ↔ worktree wiring.
- **Done:** file a ticket → a branch appears with the change. **Demo:** ticket in,
  branch out (single role).

### Phase 3 — The squad
Multiple roles, handoffs, dependencies — a team, not one agent.
- PO splits a request into tickets → assigns → engineer implements → QA reviews/tests
  → approval gate → done. Add Designer / Frontend / DevOps. Dependency resolution.
- Agent plans shown as progress on tickets; out-of-scope work proposed as tickets
  through a board tool (ARCHITECTURE "Plans are notes").
- **Retires:** coordination without chaos (board as the only channel).
- **Done:** one feature request → several roles collaborate → a reviewed branch.
  **Demo:** "one sentence in, a coordinated PR out."

### Phase 4 — Robustness & human-in-the-loop
What real projects need to trust it.
- Approval-gate UX, steering/interrupts, budget/cost controls, failure recovery,
  observability (what is each agent doing, how much memory it holds), parallel
  worktrees, the team-lead console in the terminal and the desktop app, per-role
  engine mixing.
- **Retires:** losing control on a big project.
- **Done:** several tickets in parallel, interruptible, fully visible on the board.

### Phase 5 — Extensibility & ecosystem
Make it something others build on.
- Plugin roles & custom toolsets, more engines over ACP (Gemini, opencode, local),
  equipe as an ACP agent (drive a squad from Zed or JetBrains), presets/templates,
  docs site, launch.
- Distribution: prebuilt binaries, Homebrew, `cargo install equipe`.
- **Done:** a newcomer installs equipe in one command and stands up their own squad.

## Cross-cutting (from day one)
Dogfooding · tests · safety (approval gates, worktree isolation, honest identity) ·
a terminal that's a pleasure to use · a killer README and demo.

**Tests.** `cargo test --workspace` runs with no network and no login: protocol
and wire formats (token claims, SSE, turn folding, the Claude Code stream) are pure
functions, and processes are exercised against stand-ins (a fake `claude`, scratch
git repos, a real loopback socket). What only a real account can prove — a backend
changing shape — is verified by running it, and each bug found that way gets a test.

## Decisions log

- **2026-10-11 — all-Rust.** The core moves to Rust so the desktop app can embed
  it and ship small (12 MB binary, 33 MB idle in the spike). The TypeScript CLI is
  retired from `main`; its last version is the `phase1b-try` branch.
- **2026-10-11 — Claude Code is "bring your own".** equipe runs the user's
  unmodified `claude` and never handles Claude credentials (Anthropic's terms).

## Current status

Phases 0 and 1 done (Codex edits over ACP outstanding). Phase 2 under way: the core
speaks in events and the terminal team console works; the board is next.

Open risks: whether driving `claude -p` on a subscription fits Anthropic's
allowance (ask before advertising it); Claude Code making `--bare` the default for
`-p`; GPUI's fork landscape and CJK IME quality for the desktop app.
