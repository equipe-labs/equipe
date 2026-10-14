# equipe

### Spin up a full software squad of AI agents — and ship real projects, and the work around them.

<p align="center">
  <img src="demo/equipe.gif" alt="equipe CLI" width="820">
</p>

<p align="center">
  <em>PM · Designer · Frontend · Backend · QA · Data</em> — a coordinated team,
  working off a shared board, in your repo.
</p>

<p align="center">
  <code>MIT</code> · <strong>building in public from commit #1</strong> ·
  <a href="docs/ROADMAP.md">roadmap</a> · <a href="docs/ARCHITECTURE.md">architecture</a>
</p>

> **Status: early.** A squad already lives in a background daemon on your own
> plans: you tell the PM what you want, it puts tickets on the board, engineers
> pick them up and work on their own branches inside a sandbox, asking you before
> anything beyond it, and you accept the work. Handoffs and a phone channel are
> next. Watch it build itself.

## Why

A single coding agent is great in a chat window and falls apart on a real project:
no one owns the spec, nothing gets reviewed, there's no division of labor.

equipe isn't another agent loop — mature ones already exist. It's the **team layer
on top of them**: roles, a board as the single source of truth, isolated git
worktrees for parallel work, and human approval gates where they matter.

## How it works

```
                    You ── talk to the PM, or to anyone directly
                     │
      ┌──────────────▼──────────────┐   equipe's rules, in code:
      │  PM — plans, splits tickets │   assignment · order · budgets · approval gates
      └──────────────┬──────────────┘
                     │  reads / writes
           ┌─────────▼─────────┐   ← single source of truth (SQLite + git)
           │      Board        │      tickets · status · decisions · artifacts
           └─────────┬─────────┘
      ┌─────────┬────┴─────┬─────────┬─────────┐
   Designer  Frontend   Backend     QA       Data
   each = [engine] + [role charter] + [its own memory] + [git worktree]
```

- **The board is the source of truth.** Agents coordinate through tickets and
  immutable events — not a firehose of chat. Every action ends in a visible outcome.
- **Roles are data.** The PM turns what you want into tickets; engineers edit code
  (web, backend, mobile) in isolated worktrees; QA reviews; the data analyst answers with numbers. Each
  member gets a model suited to its role, and keeps its own memory.
- **Not only code.** Research, writing, numbers, paperwork, a few steps in an app:
  the PM does a short one right in the conversation and puts a bigger one on the
  board as a `task` or `action` ticket, worked in the squad's own folder and accepted
  like code — with the files it leaves and the sources it cites on the review card.
  Any role can take one; when no one on the squad fits, the PM does it or suggests
  `/add <role>` — or `/add custom writer` (`--edits` lets a custom role change a
  project's files too).
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
| `equipe` | **The squad console**, from any folder. The first time it sets up your squad in under a minute — no repository needed; after that it opens the squad of the project you're in, else the one you used last (`--squad <name>` picks one), and in a repository no squad has it asks once whether to add it (`n` is remembered). Then: your squad on the left, the selected member's conversation and work on the right. Tell the PM what you want — it puts tickets on the board, equipe hands each to its owner, engineers work on their own branches and hand the work in; you `/accept` or `/reopen` it (Ctrl-B shows the board). Talk to anyone with Tab or `@name` — the PM hears about it. In **auto** mode members install, build and test inside a sandbox around their worktree; anything beyond — a push, a deploy — waits for your `y` / `a` / `n` (`/mode` switches between read-only, ask me, auto and full access). **The squad lives in a background daemon** — leave, or close the terminal, and it keeps working; run `equipe` again to come back |
| `equipe browser` | A browser window where **you** sign in — or sign up — to the services your squad needs: Vercel, PostHog, GitHub… Every member has a headless browser of its own from the start — signed in as you wherever you signed in — and sets things up there itself (a project, a domain, a key). They ask you before creating accounts, accepting terms, paying or deleting anything, and leave passwords, codes and CAPTCHAs to you. Needs Node.js and Google Chrome; In the console, Ctrl-W (or `/browser watch`) shows the member you talk to's browser live above its conversation, without a window; `/browser` puts that member's browser on the Mac's screen — one at a time, so thirty members never mean thirty windows — where you watch it work and sign in without closing anything; a member stuck on something you must see calls for it itself. `/browser signin` opens the separate sign-in window, `--forget` signs everyone out. Code a member runs in its browser's own process asks you first in auto, showing the code |
| `equipe daemon status\|stop\|restart` | The daemon itself. `equipe` starts it when needed; stopping it keeps open sessions, which resume when it starts again. `restart --when-idle` waits until no member is mid-turn — what to run after `equipe update`. `status` says who is working |
| `/scale` | When several of a role's tickets are ready at once and its members are busy, equipe brings in extra members of that role (`backend-2`, `backend-3`…) — up to 2 more per role by default, 6 in all — each on its own branch and worktree, sharing the role's memory; they leave after ten idle minutes. `/scale <0-6>` sets how many, `/scale off` none. Members also hand long or independent side work to subagents and keep only their summaries |
| `/send-apps` | Where a member on the desktop may send something in your name — Microsoft Teams, Slack, Mail, Outlook, Messages, KakaoTalk, Zoom, Discord… A message is typed as a draft, but Return or the Send button waits for your yes every time, in every mode, showing the draft, the conversation and the screen (`o` opens it). Other apps never ask; payment, account deletion, passwords and security settings take no input at all. `/send-apps add <app>`, `remove <app>`, `reset` |
| `Ctrl-V` · drop | Ctrl-V attaches the clipboard's image (a screenshot) to your message; dropping an image file on the terminal attaches it too. The member looks at it before answering. **Over SSH**, a terminal only carries text, so the other machine can't see your Mac's clipboard: run `equipe paste-setup` once on the Mac you type on (or `curl -fsSL https://github.com/equipe-labs/equipe/releases/latest/download/paste-setup.sh \| sh` if equipe isn't installed there) and ⌃⇧V in iTerm2 types the clipboard's screenshot down the connection, where equipe attaches it (`--remove` undoes it). Without it, Ctrl-V over SSH opens a picker on your Mac, and `equipe send-image <host>` sends the clipboard image from your Mac |
| `/find` · `/tidy` | A board of hundreds stays readable: work not yet started folds past ten per section (`e` shows it whole), done work keeps its latest few, and **f** on the board (or `/find <words>`) narrows it as you type — titles, notes and T-numbers, done work too; Esc clears. `g` shows open work as a tree of what waits for what, and each ticket says what it's after and what it unblocks. Once a day the PM hears which unstarted tickets haven't moved in a week and proposes, next time you talk, what to close, merge or keep — it never drops work itself; `/tidy` asks now |
| `/model` | `/model <member> <model>` changes the model a member works on from its next turn — Claude Code's `opus` / `sonnet` / `haiku`, or any model its engine offers (Codex's, and the engines to come); its conversation carries on. Completion lists what the member's engine takes |
| `/usage` | The footer shows your Claude plan's five-hour and weekly windows; `/usage` adds when they start over, and each member's context, tokens processed and the share from cache |
| `/compact` · `/clear` | Conversations stay a sensible size on their own: Claude Code compacts a long one as it fills, and a member's desk conversation (between tickets) starts afresh — handed a summary of where the last one left off — once it is 60% full, or after an hour unused, when carrying it on would mean processing all of it again. `/compact [member]` asks for that now; `/clear [member]` starts clean. Memory and the board carry over either way. The title shows how full the selected member's context is, and token counts include cached input |
| `equipe pair` · `equipe devices` | **The equipe app on your phone.** `equipe pair` shows a QR code; scan it in the app, say `y` at the terminal, and the phone follows your squads from anywhere — requests for permission (Face ID, then once — never "always"), work to review, the board, the PM — with a push when something needs you. Phone and machine reach each other through the equipe relay (`relay/`, on Cloudflare), every frame sealed between the two ends (HPKE): the relay passes envelopes it can't open, and nothing listens on your machine. `equipe devices` lists paired phones; `equipe devices remove <name>` cuts one off at once. Protocol: `docs/remote-protocol.md` |
| `equipe telegram setup` | **Your squads from your phone.** Make a bot with @BotFather, paste its token, tap the link it shows: the bot is paired with your Telegram account and answers no one else. Requests for permission arrive with Allow / Decline (once — never "always" from a phone), work to review with Accept / Ask for changes, tickets waiting on you with a reply box; write to talk to the PM, `@name …` for a member, reply to a member's message to go on with them; `/status`, `/board`, `/squad`. Telegram delivers the notifications. Runs in the daemon; `equipe telegram off` disconnects. Messages travel through Telegram's servers (encrypted to them, not end to end) |
| `equipe daemon install\|uninstall` | Run the daemon as a login service (launchd on macOS, systemd on Linux): it starts when you log in, comes back after a crash, and outlives the terminal or SSH session that started it |
| `equipe login` | Pick an engine and sign in (`codex` / `claude`, `--again`, `--no-browser`) |
| `equipe logout` | Forget equipe's Codex login (Claude Code keeps its own) |
| `equipe status` | Which engines are ready, on which plan, with which models |
| `equipe roles` | The roles a squad can have, what each does and the model it gets |
| `equipe squads` | Your squads and the projects each works on |
| `/squad` · `/project` | In the console: `/squad <name>` switches to another squad (click its name in the header), `/squad new <name> [folder]` starts one, `/squad add <folder>` / `/squad remove <project>` change what a squad works on. The console is about the whole squad, wherever you run it — in one of its repositories or not, the same screen opens: the header shows the squad, its home folder, and a chip per project with who is at work on it (●) and what waits for you there (⚑), then **all**, where it starts. The board tags every ticket with its project — `home` for work in the squad home — and its kind (`code`, `task`, `action`), and with work in several places gives each a lane below what waits for you; members show the project they work in. `/project <name>`, a click on its chip, or **Ctrl-P** (the next one, then all; Alt-1…9 where the terminal sends Alt, Alt-0 all) filters to one project — the board, the review card and new requests are about it, and the line under the composer says so; `/project all` takes the filter off. Without a filter, what you tell the PM is for the squad, and the PM decides where it goes. Adding an app suggests a mobile engineer (`/add mobile`) **Every member keeps a conversation per project, and one for the whole squad**: filtering to a project switches who you talk to into its thread, so what's said about one project never crowds another's (the squad's memory is shared by all). The console looks through the same lens — a member's work on other projects folds into one line saying so, each ticket is headed with its project when the whole squad is in view, and a message to a member at work elsewhere says where it goes. Click a member in the squad to talk to it |
| `equipe try "…"` | One turn on whichever engine is ready (`--engine`, `--model`) |
| `equipe try --edit "…"` | An agent edits your repo **on a new branch**, in its own worktree. Your checkout is untouched; you get review / keep / discard commands |

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

macOS and Linux:

```bash
curl -fsSL https://github.com/equipe-labs/equipe/releases/latest/download/install.sh | sh
```

That puts a prebuilt `equipe` in `~/.local/bin` (checked against its SHA-256)
and adds it to your PATH. Or with Homebrew:

```bash
brew install equipe-labs/tap/equipe
```

Then, from any folder:

```bash
equipe
```

The first run sets up your squad — no repository needed. After that, `equipe`
opens your squad wherever you run it: inside one of its projects, that project's
squad; anywhere else, the squad you used last. Run it in a repository no squad
has yet and the console asks once whether to add it. Either way it opens on the
whole squad — every project and the squad home — not on the folder you ran it
in; a project's chip filters to it. Members run on
[Claude Code](https://code.claude.com/docs/en/setup) with your own plan, or on a
ChatGPT plan with `equipe login codex`.

**Update** with `equipe update` (or `brew upgrade equipe`). When a squad is running, `equipe daemon restart`
once no one is working puts it on the new build.

<details>
<summary>From source</summary>

Needs [Rust](https://rustup.rs) 1.88+ and git.

```bash
git clone https://github.com/equipe-labs/equipe && cd equipe
cargo install --locked --path crates/cli
```

</details>

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
