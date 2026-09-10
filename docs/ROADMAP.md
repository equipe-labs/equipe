# Equipe roadmap

How we build equipe. This is the canonical plan; `docs/ARCHITECTURE.md` covers the design.

## Five principles

1. **Walking skeleton first.** Build one thin path that pierces every layer, then
   thicken it. Don't widen before the spine works end to end.
2. **De-risk the scary part first.** The real unknown isn't the board or the CLI —
   it's *"an ACP-driven coding agent, running on a subscription token, edits real
   files."* If that doesn't hold, the premise doesn't. Prove it before plumbing.
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
- **Done:** `equipe login` (ChatGPT/Codex subscription OAuth), `equipe roles`.

### Phase 1 — Prove the pipe (spike) ← now
One agent, running on the subscription, does real work. No board yet.
- **1a — token reaches the model.** `equipe status`: refresh the stored token if
  stale, call the Codex backend, print the account + available models. Retires the
  biggest unknown (auth → backend) at the smallest surface.
- **1b — the model does real work.** `equipe try "<prompt>"`: one turn that produces
  output; then one turn that **edits a file** in a repo through an engine.
- **Retires:** does subscription-driven execution actually work end to end?
- **Done:** `equipe status` lists models on a real login; `equipe try` makes a real
  edit. **Demo:** "logged in with my ChatGPT plan, an agent edited my repo."

### Phase 2 — The spine
Board + one role + one ticket → a real branch/diff.
- SQLite board (`tickets`, `events`) + `equipe board`.
- One `backend` role (charter + toolset) + an orchestrator that takes a ticket,
  spins a git worktree, runs the engine, and lands a branch. Human reviews.
- **Retires:** orchestrator ↔ board ↔ worktree wiring.
- **Done:** file a ticket → a branch appears with the change. **Demo:** ticket in,
  PR out (single role).

### Phase 3 — The squad
Multiple roles, handoffs, dependencies — a team, not one agent.
- PO splits a request into tickets → assigns → engineer implements → QA reviews/tests
  → approval gate → done. Add Designer / Frontend / DevOps. Dependency resolution.
- **Retires:** coordination without chaos (board as the only channel).
- **Done:** one feature request → several roles collaborate → a reviewed PR.
  **Demo:** "one sentence in, a coordinated PR out."

### Phase 4 — Robustness & human-in-the-loop
What real projects need to trust it.
- Approval-gate UX, steering/interrupts, budget/cost controls, failure recovery,
  observability (what is each agent doing?), parallel worktrees, a live board
  view (TUI or web), per-role provider mixing.
- **Retires:** losing control on a big project.
- **Done:** several tickets in parallel, interruptible, fully visible on the board.

### Phase 5 — Extensibility & ecosystem
Make it something others build on.
- Plugin roles & custom toolsets, more engines (Claude Code, Gemini, opencode,
  local), presets/templates, full dogfooding, docs site, launch.
- **Done:** `npm i -g equipe` and a newcomer stands up their own squad.

## Cross-cutting (from day one)
Dogfooding · tests · safety (approval gates, worktree isolation, honest identity) ·
a killer README and demo.

## Current status
Phase 0 done. Phase 1 in progress (starting with `equipe status`).
