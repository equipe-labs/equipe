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

> **Status: early.** One agent can already edit your repo on its own branch, on
> your own plan. The squad — board, roles, handoffs — is next. Watch it build itself.

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
- **Roles are data.** The PO writes specs; engineers edit code in isolated
  worktrees; QA runs tests; DevOps ships.
- **Engines are borrowed, not rebuilt.** equipe drives mature coding agents —
  Codex and Claude Code today, more over the
  [Agent Client Protocol](https://agentclientprotocol.com) — and mixes them per role.
- **You stay in the loop** at approval gates (e.g. before a branch merges).

## What works today

```
$ equipe status
equipe.  v0.0.1

CODEX  ChatGPT plan · equipe signs in
  account    …
  backend    ✓ 3 models  gpt-6-luna, gpt-5.6-terra, gpt-5.6-luna

CLAUDE CODE  your claude · signs itself in
  login      ✓ claude.ai · max subscription

  ready      codex · claude
```

| Command | What it does |
|---|---|
| `equipe` | **The team console.** A full-screen terminal UI: your agents on the left, the selected one's conversation and work on the right. `/new` opens another agent, `/edit` one that changes files on its own branch, `/close` lands that branch. **The team lives in a background daemon** — leave, or close the terminal, and it keeps working; run `equipe` again to come back |
| `equipe daemon status\|stop\|restart` | The daemon itself. `equipe` starts it when needed; stopping it keeps open sessions, which resume when it starts again |
| `equipe login` | Pick an engine and sign in (`codex` / `claude`, `--again`, `--no-browser`) |
| `equipe logout` | Forget equipe's Codex login (Claude Code keeps its own) |
| `equipe status` | Which engines are ready, on which plan, with which models |
| `equipe try "…"` | One turn on whichever engine is ready (`--engine`, `--model`) |
| `equipe try --edit "…"` | An agent edits your repo **on a new branch**, in its own worktree. Your checkout is untouched; you get review / keep / discard commands |
| `equipe roles` | The squad roles |

```
$ equipe try --edit "In app.py, capitalize the name in greet()"
claude code · may edit files
  branch     equipe/try-6acb8840  from b54a531

· read   app.py
✎ edit   app.py
Done. greet() now capitalizes the name.

✓ Changes committed to equipe/try-6acb8840
  review     git show equipe/try-6acb8840
  keep       git merge equipe/try-6acb8840
  discard    git branch -D equipe/try-6acb8840
```

## Engines & sign-in

Both engines run on a plan **you** already have. equipe never pays for, resells or
pools anyone's usage.

**Codex — equipe signs in.** `equipe login codex` runs OAuth so equipe can use
**your** ChatGPT plan, with PKCE and the public Codex OAuth client the same way
OpenAI's own Codex CLI does. equipe **identifies itself honestly** as a
third-party harness (`originator: equipe`) and never impersonates the official CLI.

> Whether a consumer subscription fits OpenAI's Terms this way is your call and
> your risk.

**Claude Code — bring your own.** equipe runs **your installed, unmodified
`claude`**, which signs itself in on Anthropic's own page (`equipe login claude`
just starts that flow). equipe never reads, stores or forwards Claude credentials:
Anthropic's terms don't allow third-party tools to offer Claude.ai login or handle
its tokens, and they do allow you to sign in to the unmodified Claude Code binary
with your own plan. Usage is billed to you under your own agreement; `equipe status`
says whether that's a subscription or per-token API billing.

> Anthropic's advertised Pro/Max limits assume ordinary, individual use. A whole
> squad on one subscription will reach them; plan to give Claude Code the roles that
> need it most.

## Install

From source, until releases are published. Needs [Rust](https://rustup.rs) 1.88+
and git; [Claude Code](https://code.claude.com/docs/en/setup) for the `claude` engine.

```bash
git clone https://github.com/livelikeabel/equipe && cd equipe
cargo install --locked --path crates/cli
```

`cargo install` puts `equipe` in `~/.cargo/bin`. To install elsewhere on your PATH,
add `--root ~/.local` (→ `~/.local/bin/equipe`). Run the same command again to
update.

## Development

```bash
cargo test --workspace                     # no network, no login needed
cargo clippy --workspace --all-targets
cargo run -p equipe -- status              # the CLI from source
cargo run --release -p equipe-desktop      # the desktop app (see docs/DESKTOP.md)
demo/render.sh                             # regenerate the hero gif (see demo/README.md)
```

| Crate | |
|---|---|
| `crates/protocol` | `equipe-protocol` — the contract between the brain and every screen: Commands in, Events out |
| `crates/core` | `equipe-core` — the brain: the runtime that runs sessions on engines, roles, the board, worktrees |
| `crates/providers` | `equipe-providers` — sign-in and engines: Codex, your Claude Code |
| `crates/cli` | `equipe` — the command line and the terminal UI |
| `crates/desktop` | `equipe-desktop` — the desktop app (GPUI) |

## License

MIT
