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

### Phase 2 — The resident squad ← now
The team lives in a daemon, not in a terminal: closing the terminal never stops
it, and every surface sees the same team. Then the team itself — members with
roles, a board, a director.

1. ✅ **Daemon + local gateway.** The core runtime moves behind a Unix socket
   (`~/.equipe/equipe.sock`, JSON-RPC 2.0 carrying the protocol's Commands and
   Events). `equipe` attaches — starting the daemon if needed — and detaches
   without stopping anything; reattaching catches up from a snapshot. Agents are
   the daemon's children, not the terminal's. What happened is journaled in
   SQLite under the storage policy below, so a restarted daemon resumes its
   sessions and their conversations.
2. **Resident members, board, director.** ← next. A member is a role (pm, designer,
   frontend, backend, qa, data, or custom) + an engine + a model + memory + a
   worktree; models are recommended per role by tier, resolved against the
   account's live models, with QA on a different engine from the implementer.
   The SQLite board holds tickets and their record; the director turns a request
   into tickets and assigns them. `equipe init` stands up a default squad.
3. **First channel: Telegram.** Notifications, approve/deny buttons, short
   instructions to the PM — through the daemon's outbound connection, no open
   port. Senders must be paired; irreversible steps always need a button press.
4. **Desktop app** — another client of the same socket.

- ✅ **Core speaks in events** (`equipe-protocol` + the core runtime).
- ✅ **Interactive `equipe` v0**: the team console — several agents at once,
  streaming, conversation memory, bracketed paste, `/new` `/edit` `/close`.
- **Retires:** a team that outlives its terminal; orchestrator ↔ board ↔ worktree.
- **Done:** close the terminal mid-task, reopen it, the work is done or still going;
  one request becomes tickets that members land as branches. **Demo:** "I closed my
  laptop's terminal; my squad kept shipping."

### Phase 3 — The squad at work
Handoffs, dependencies, review — a team, not parallel agents.
- PM splits a request into tickets → assigns → engineer implements → QA reviews/tests
  → approval gate → done. Dependency resolution.
- Agent plans shown as progress on tickets; out-of-scope work proposed as tickets
  through a board tool (ARCHITECTURE "Plans are notes").
- Codex edits over ACP (`codex-acp`) with the login-once handoff.
- **Retires:** coordination without chaos (board as the only channel).
- **Done:** one feature request → several roles collaborate → a reviewed branch.
  **Demo:** "one sentence in, a coordinated PR out."

### Phase 4 — Robustness & human-in-the-loop
What real projects need to trust it.
- Approval UX on every surface, steering/interrupts, budget/cost controls, failure
  recovery, observability (what is each agent doing, how much memory it holds),
  start at login (launchd/systemd), the daemon on a server (attach over SSH), more
  channels (Slack, Discord).
- **Retires:** losing control on a big project.
- **Done:** several tickets in parallel, interruptible, visible everywhere.

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

**Storage.** equipe journals the *record* and the *conversation*, never the raw
material. Kept forever: tickets, assignments, decisions, approvals, branches landed
— a few hundred bytes each. Kept for a window (default 90 days, then reduced to a
one-line summary): messages, finished replies (never per-token deltas), one line per
tool use, tokens. Never kept: file contents, command output, images, thinking,
diffs — git holds the changes and each engine keeps its own full transcript, which
equipe only references. One database per project, incremental vacuum so pruning
returns disk, a size cap, and `equipe storage` / `equipe prune` to see and act. (For
scale: on the author's machine Claude Code wrote 1.8 GB of transcripts in 25 days;
the human-readable conversation in its largest session was 3.4% of its size.)

## Decisions log

- **2026-10-11 — all-Rust.** The core moves to Rust so the desktop app can embed
  it and ship small (12 MB binary, 33 MB idle in the spike). The TypeScript CLI is
  retired from `main`; its last version is the `phase1b-try` branch.
- **2026-10-11 — Claude Code is "bring your own".** equipe runs the user's
  unmodified `claude` and never handles Claude credentials (Anthropic's terms).
- **2026-10-12 — a resident daemon now, not later.** The squad must outlive the
  terminal and reach a phone soon, so the gateway moves from Phase 4 to the front
  of Phase 2. Inside the daemon the core is still one Rust process; screens and
  channels are clients. First channel: Telegram.

## Current status

Phases 0 and 1 done (Codex edits move to Phase 3, over ACP). Phase 2 under way: the
team lives in a daemon — the console detaches and reattaches, a closed terminal
stops nothing, and a restarted daemon resumes its sessions and conversations.
Resident members, the board and the director are next.

Open risks: whether driving `claude -p` on a subscription fits Anthropic's
allowance (ask before advertising it); Claude Code making `--bare` the default for
`-p`; GPUI's fork landscape and CJK IME quality for the desktop app.
