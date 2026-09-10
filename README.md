# equipe

### Spin up a full software squad of AI agents — and ship real projects.

<p align="center">
  <img src="demo/equipe.gif" alt="equipe CLI" width="820">
</p>

<p align="center">
  <em>PO · Designer · Frontend · Backend · QA · DevOps</em> — a coordinated team,
  working off a shared board, in your repo.
</p>

<p align="center">
  <code>MIT</code> · <strong>building in public from commit #1</strong> ·
  <a href="docs/ROADMAP.md">roadmap</a> · <a href="docs/ARCHITECTURE.md">architecture</a>
</p>

> **Status: early.** The spine is landing in the open — not production-ready yet.
> Watch it build itself.

## Why

A single coding agent is great in a chat window and falls apart on a real project:
no one owns the spec, nothing gets reviewed, there's no division of labor.

equipe isn't another agent loop — mature ones already exist. It's the **team layer
on top of them**: roles, a board as the single source of truth, isolated git
worktrees for parallel work, and human approval gates where they matter.

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
  PO      Designer  Frontend  Backend    QA       DevOps
   each = [engine] + [role charter] + [toolset] + [git worktree]
```

- **The board is the source of truth.** Agents coordinate through tickets and
  immutable events — not a firehose of chat. Every action ends in a visible outcome.
- **Roles are `charter + toolset + permissions`.** The PO writes specs; engineers
  edit code in isolated worktrees; QA runs tests; DevOps ships.
- **Engines are swappable (ACP-first).** equipe drives mature coding agents over the
  [Agent Client Protocol](https://agentclientprotocol.com) — Codex, Claude Code,
  Gemini, opencode — and uses thin LLM calls for non-coding roles.
- **You stay in the loop** at approval gates (e.g. before a PR merges).

## Models & auth

Model-agnostic through a provider abstraction. The **default** path is the one you
already pay for:

- `equipe login` runs OAuth so the squad runs on **your own** ChatGPT Plus/Pro (Codex)
  subscription — no API key required.
- API keys and other providers (Anthropic, local models) are first-class and mixable
  **per role** — put low-volume roles on the subscription, heavy code-gen on an API key.

> equipe uses OpenAI's public Codex OAuth client the same way OpenAI's own Codex CLI
> does, authenticates *your* account, and **identifies itself honestly** as a
> third-party harness (it never impersonates the official CLI). Whether a consumer
> subscription fits OpenAI's Terms this way is your call and your risk. If in doubt,
> use an API key.

## Quick start

```bash
npm install -g equipe      # (not published yet — building in the open)

equipe login               # authenticate (ChatGPT/Codex OAuth by default)
equipe status              # verify the login, list available models
equipe roles               # see the squad
```

## Development

```bash
pnpm install
pnpm dev -- roles          # run the CLI from source
pnpm build                 # compile to dist/
pnpm demo                  # regenerate the hero gif (see demo/README.md)
```

Requires Node 22+.

## License

MIT
