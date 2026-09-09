# equipe

**Spin up a full software squad of AI agents — and let them build the real thing.**

`equipe` (French for *team*, pronounced "eh-KEEP") assembles a coordinated crew of
role agents — **PO · Designer · Frontend · Backend · QA · DevOps** — that pull work
off a shared board and build large, real projects together. You bring the intent and
the approvals; the squad does the work in your git repo.

> Status: **early skeleton.** The architecture and CLI surface are landing first;
> the orchestration and engines are being built in the open. Not production-ready.

## Why

Single coding agents are great in a chat window and fall apart on a real project:
no one owns the spec, nothing gets reviewed, and there's no division of labor.
`equipe` is not another agent loop — mature agent loops already exist. It is the
**team layer on top of them**: roles, a board as the single source of truth,
git worktrees for parallel work, and human approval gates at the boundaries that matter.

## How it works

```
             ┌──────────────── Orchestrator (Director) ────────────────┐
             │  plan · split into tickets · resolve deps · approval gates │
             └───────────────┬─────────────────────────────────────────┘
                             │  reads / writes
                   ┌─────────▼─────────┐   ← single source of truth (SQLite + git)
                   │      Board        │      tickets · status · decisions · artifacts
                   └─────────┬─────────┘
        pull ↑ tickets       │  events / handoffs
   ┌────────┬────────┬───────┴──┬─────────┬─────────┐
  PO      Designer  Frontend  Backend    QA       DevOps     ← role agents
   each = [engine] + [role charter] + [toolset] + [git worktree]
```

- **Board is the source of truth.** Agents coordinate through tickets and immutable
  events, not a firehose of chatter. Every action ends in a visible outcome on the board.
- **Roles are `charter + toolset + permissions`.** The PO writes specs; engineers edit
  code in isolated worktrees; QA runs tests; DevOps ships.
- **Engines are swappable (ACP-first).** Equipe drives mature agents over the
  [Agent Client Protocol](https://agentclientprotocol.com) (Codex, Claude Code, Gemini,
  opencode, …) or a local CLI — and uses thin LLM calls for non-coding roles.
- **Humans stay in the loop** at approval gates (e.g. before a PR merges).

## Models & auth

Equipe is model-agnostic through a provider abstraction. The **default** path is the
one people already pay for:

- **OpenAI ChatGPT Plus/Pro (Codex) subscription — via OAuth.** `equipe login` runs the
  standard OAuth flow so the squad runs on **your own subscription**, no API key required.
- API keys and other providers (Anthropic, local models) are first-class and can be mixed
  **per role** — put low-volume roles (PO/QA) on the subscription and heavy code generation
  on an API key or a local model.

> **On subscription use — read this.** Equipe uses OpenAI's public Codex OAuth client the
> same way OpenAI's own Codex CLI does; it authenticates *your* account and identifies
> itself honestly as a third-party harness (it does **not** impersonate the official CLI).
> Whether using a consumer subscription this way fits OpenAI's Terms is your call and your
> risk — Equipe never hides what it is. If in doubt, use an API key.

## Quick start

```bash
npm install -g equipe      # (not published yet — building in the open)

equipe login               # authenticate (ChatGPT/Codex OAuth by default)
equipe init                # create equipe.config.jsonc in your repo
equipe run                 # let the squad start pulling tickets
```

## Development

```bash
pnpm install
pnpm dev -- roles          # run the CLI from source
pnpm build                 # compile to dist/
```

Requires Node 22+.

## License

MIT
