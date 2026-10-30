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
   each = [engine] + [role charter] + [its role's memory] + [git worktree]
```

- **The board is the source of truth.** Agents coordinate through tickets and
  immutable events — not a firehose of chat. Every action ends in a visible outcome.
- **Roles are data.** The PM turns what you want into tickets; engineers edit code
  (web, backend, mobile) in isolated worktrees; QA reviews; the data analyst answers with numbers. Each
  member gets a model suited to its role, and learns into its role's memory —
  shared by every member of the role, so what `backend-3` learned in a rush stays
  with `backend` after it leaves.
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
| `equipe` | **The squad console**, from any folder. The first time it sets up your squad in under a minute — no repository needed; after that it opens the squad of the project you're in, else the one you used last (`--squad <name>` picks one), and in a repository no squad has it asks once whether to add it (`n` is remembered). Then: **one conversation with the PM, and the work as tickets.** On the left, the PM, then every ticket by where it stands — waiting for you, in progress, to do, done — then the team; on the right, what's open. Tell the PM what you want — it answers what's quick and puts the rest on tickets (its own work too, so it's never out of reach), equipe hands each to its owner, engineers work on their own branches and hand the work in. Open a ticket (Tab, or a click) to see its work and steer it as it goes; `/accept` or `/reopen` it (Ctrl-B shows the board). `@name` talks to someone directly — the PM hears about it. In **auto** mode members install, build and test inside a sandbox around their worktree; anything beyond — a push, a deploy — waits for your `y` / `a` / `n` (`/mode` switches between read-only, ask me, auto and full access). **The squad lives in a background daemon** — leave, or close the terminal, and it keeps working; run `equipe` again to come back |
| `equipe browser` | **One Chrome for the whole squad, signed in once.** equipe runs a Google Chrome of its own on this Mac — an ordinary one as sites see it, so Google lets you in — and `equipe browser` opens a window in it where **you** sign in, or sign up, wherever the squad should work: Google, GitHub, Vercel, PostHog… Once is enough: every member works in that Chrome, each conversation in a window of its own that sees only its own tabs and the popups it opened — a member talking about two projects at once has two — side by side and at the same time, signed in as you — no copies of your sign-ins, nothing to repeat per member. Members' windows open behind what you're doing and work minimized; each glows in the member's colour with its name (and the project, for a conversation beside its own) at the foot, and outlines what it clicks or types into. They ask you before creating accounts, accepting terms, paying or deleting anything, and leave passwords, codes and CAPTCHAs to you. Needs Node.js and Google Chrome. In the console, Ctrl-W (or `/browser watch`) shows the member you talk to's window live above its conversation; `/browser` brings that member's window to the front of the Mac's screen — one at a time — where you watch it work or sign in for it; a member stuck on something you must see calls for it itself. `/browser signin` opens your window. Code a member runs in its browser's own process asks you first in auto, showing the code |
| `equipe desktop setup` | **Experimental, macOS.** Members work this Mac's desktop apps the way they work the browser — screenshot, read an app's buttons and fields, click, type (Korean included), shortcuts, open and bring apps forward. A small helper app, *equipe Desktop Helper*, is the only thing you give Screen Recording and Accessibility to (equipe and its members never get them); `setup` installs it, opens the two Settings panes and says what is still missing — Accessibility takes your password or Touch ID to switch on. Off for every member until `equipe desktop on <member> --experimental` (`off` takes it back, `status` checks, `remove` uninstalls). Each click is checked against the app the member means, typing is refused when the keyboard focus is elsewhere and stops if it moves — leave the Mac alone while a member works it. Screenshots are never saved by equipe (Claude Code's own transcript keeps them) |
| `equipe daemon status\|stop\|restart` | The daemon itself. `equipe` starts it when needed; stopping it keeps open sessions, which resume when it starts again. `restart --when-idle` waits until no member is mid-turn — what to run after `equipe update`. `status` says who is working |
| `/add` | Teammates are yours to add: `/add <role…>` brings one in, `/add custom <name>` one of your own making, and the PM proposes one when a role's work keeps piling up — equipe never adds members on its own. Big work runs in parallel inside a member instead: it splits a ticket that divides cleanly across subagents and runs them together, each shown under its name; long or independent side work goes to subagents too, and the member keeps only their summaries |
| `/send-apps` | Where a member on the desktop may send something in your name — Microsoft Teams, Slack, Mail, Outlook, Messages, KakaoTalk, Zoom, Discord… A message is typed as a draft, but Return or the Send button waits for your yes every time, in every mode, showing the draft, the conversation and the screen (`o` opens it). Other apps never ask; payment, account deletion, passwords and security settings take no input at all. `/send-apps add <app>`, `remove <app>`, `reset` |
| `Ctrl-V` · drop | Ctrl-V attaches the clipboard's image (a screenshot) to your message; dropping an image file on the terminal attaches it too. The member looks at it before answering. **Over SSH**, a terminal only carries text, so the other machine can't see your Mac's clipboard: run `equipe paste-setup` once on the Mac you type on (or `curl -fsSL https://github.com/equipe-labs/equipe/releases/latest/download/paste-setup.sh \| sh` if equipe isn't installed there) and ⌃⇧V in iTerm2 types the clipboard's screenshot down the connection, where equipe attaches it (`--remove` undoes it). Without it, Ctrl-V over SSH opens a picker on your Mac, and `equipe send-image <host>` sends the clipboard image from your Mac |
| `/find` · `/tidy` | A board of hundreds stays readable: work not yet started folds past ten per section (`e` shows it whole), done work keeps its latest few, and **f** on the board (or `/find <words>`) narrows it as you type — titles, notes and T-numbers, done work too; Esc clears. `g` shows open work as a tree of what waits for what, and each ticket says what it's after and what it unblocks. Once a day the PM hears which unstarted tickets haven't moved in a week and proposes, next time you talk, what to close, merge or keep — it never drops work itself; `/tidy` asks now |
| When it's done | A member starting anything longer than a couple of minutes says when it expects to be done, and — pushing a release, deploying — when what it ships will be live. The console shows it where you look: `done ~20:52 (12m)` and `v0.28.1 on Homebrew live ~21:05 (25m)` beside a member at work, `~12m` (or `↑25m` until live) in the squad and on the all view, and the PM hears it too. Past its time it says `still going` rather than going quiet. When a member hasn't said, equipe guesses from how long the same kind of work took before — that role's tickets of that kind (else anyone's), or its turns at the desk — and says it's a guess: `usually done ~20:52 (12m)`, `≈12m`, `longer than usual`; a guess shows once a turn has gone on a minute |
| `/suggest` | After a member answers you, your likely next message waits greyed in the empty composer — "yes, tidy the other pages too", the obvious next step — and **→** takes it; typing ignores it. Claude Code's small model guesses it from the end of the conversation alone (a few hundred tokens, no tools, nothing kept). `/suggest off` stops it, `/suggest on` brings it back |
| `/model` | `/model <member> <model>` changes the model a member works on from its next turn — Claude Code's `opus` / `sonnet` / `haiku`, or any model its engine offers (Codex's, and the engines to come); its conversation carries on. Completion lists what the member's engine takes |
| `/usage` | The footer shows your Claude plan's five-hour and weekly windows; `/usage` adds when they start over, and each member's context, tokens processed and the share from cache |
| `/compact` · `/clear` | Conversations stay a sensible size on their own: Claude Code compacts a long one as it fills, and a member's desk conversation (between tickets) starts afresh — handed a summary of where the last one left off — once it is 60% full, or after an hour unused, when carrying it on would mean processing all of it again. `/compact [member]` asks for that now; `/clear [member]` starts clean. Memory and the board carry over either way. The title shows how full the selected member's context is, and token counts include cached input |
| `equipe pair` · `equipe devices` | **The equipe app on your phone.** `equipe pair` shows a QR code; scan it in the app, say `y` at the terminal, and the phone follows your squads from anywhere — requests for permission (Face ID, then once — never "always"), work to review, the board, the PM — with a push when something needs you. Phone and machine reach each other through the equipe relay (`relay/`, on Cloudflare), every frame sealed between the two ends (HPKE): the relay passes envelopes it can't open, and nothing listens on your machine. `equipe devices` lists paired phones; `equipe devices remove <name>` cuts one off at once. Protocol: `docs/remote-protocol.md` |
| `equipe telegram setup` | **Your squads from your phone.** Make a bot with @BotFather, paste its token, tap the link it shows: the bot is paired with your Telegram account and answers no one else. Requests for permission arrive with Allow / Decline (once — never "always" from a phone), work to review with Accept / Ask for changes, tickets waiting on you with a reply box; write to talk to the PM, `@name …` for a member, reply to a member's message to go on with them; `/status`, `/board`, `/squad`. Telegram delivers the notifications. Runs in the daemon; `equipe telegram off` disconnects. Messages travel through Telegram's servers (encrypted to them, not end to end) |
| `equipe daemon install\|uninstall` | Run the daemon as a login service (launchd on macOS, systemd on Linux): it starts when you log in, comes back after a crash, and outlives the terminal or SSH session that started it |
| `equipe login` | Pick an engine and sign in (`codex` / `claude`, `--again`, `--no-browser`) |
| `equipe logout` | Forget equipe's Codex login (Claude Code keeps its own) |
| `equipe status` | Which engines are ready, on which plan, with which models |
| `equipe review` | An NVIDIA Nemotron model on Nebius Token Factory reviews this repository's changes — against `HEAD`, or `--base <ref>` — with only `NEBIUS_API_KEY` (see [Nemotron review](#nemotron-review-on-nebius-token-factory)). `--models` lists the account's Nemotron models |
| `equipe roles` | The roles a squad can have, what each does and the model it gets |
| `equipe squads` | Your squads and the projects each works on |
| `equipe lang [code\|auto]` · `/lang` | The language equipe speaks to you — ko, en, ja, zh-Hans, zh-Hant, es, fr, de — one pick for the console, the app and what the daemon says in conversations (the app picks it in Settings, live). Nothing picked (`auto`): `LANGUAGE`/`LC_ALL`/`LC_MESSAGES`/`LANG` in a terminal, then the system's preferred languages in order (on a Mac, the language set for equipe alone in System Settings › General › Language & Region › Applications included), else English. `EQUIPE_LANG` overrides everything. Members write to you in the language you write in |
| `/squad` · `/project` | In the console: `/squad <name>` switches to another squad (click its name in the header), `/squad new <name> [folder]` starts one, `/squad add <folder>` / `/squad remove <project>` change what a squad works on. The console is about the whole squad, wherever you run it — in one of its repositories or not, the same screen opens: the header shows the squad, its home folder, and a chip per project with who is at work on it (●) and what waits for you there (⚑), then **all**, where it starts. The board tags every ticket with its project — `home` for work in the squad home — and its kind (`code`, `task`, `action`), and with work in several places gives each a lane below what waits for you; members show the project they work in. `/project <name>`, a click on its chip, or **Ctrl-P** (the next one, then all; Alt-1…9 where the terminal sends Alt, Alt-0 all) filters to one project — the board, the review card and new requests are about it, and the line under the composer says so; `/project all` takes the filter off. **All** is the control tower: over the PM's conversation, a line a project says who is at it — members on its tickets, conversations about it — and what waits for you there (a click goes in), and the PM, told the same with each message, answers for the whole squad — priorities, what waits on you, what goes next — and sends work for one project to it as tickets. Adding an app suggests a mobile engineer (`/add mobile`) **Each member keeps one conversation**: the project you look at travels with what you say, and a ticket's work lives with its ticket, so no project crowds another's (the squad's memory is shared by all). Click a member in the squad to talk to it |
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

## Nemotron review on Nebius Token Factory

**What.** When a member hands in a code ticket, an **NVIDIA Nemotron** model on
**Nebius Token Factory** reads the branch's diff against the ticket and writes a
short review: a verdict (*looks good* · *needs attention* · *risky*), a summary,
up to five risks as `file:line — why`, and up to five tests the change is missing.
It shows in the desktop app's ticket peek, under the hand-in, and is kept on the
ticket's record. It is **advice only**: it never accepts, merges or sends work back —
you (or the PM, if you let it accept) still decide.

**Why.** The members that write the code (Claude Code, Codex) should not be the only
ones who read it. A second model, from another vendor and with a narrow job —
correctness bugs, security, data loss, acceptance criteria the diff doesn't meet —
gives the person a place to start before they accept.

**Where the call happens.** `crates/core/src/nemotron.rs` is the whole client:
`POST <base>/chat/completions` (and `GET <base>/models` to pick a model), OpenAI-compatible,
`Authorization: Bearer $NEBIUS_API_KEY`. The daemon calls it off its main loop when a
code ticket is handed in (`runtime.rs`, `end_work` → `ask_for_review` → `on_reviewed`),
stores the result in the squad's journal (`reviews` table) and sends
`Event::CodeReviewed` to every screen. `equipe review` calls the same client from the
command line.

**Try it with only a Nebius key** — no squad, no Claude or ChatGPT plan needed:

```bash
read -rs NEBIUS_API_KEY && export NEBIUS_API_KEY   # paste the key; it isn't echoed or kept in history
cd your-repo
equipe review --models          # the NVIDIA Nemotron models your account offers
equipe review                   # your uncommitted changes (working tree + staged) against HEAD
equipe review --base main       # what this branch changed since it left main
equipe review --base main --goal "Refuse expired tokens with 401"   # and check it against what it should do
```

New files are reviewed once they're added (`git add -N` is enough). It prints the
verdict, summary, risks, missing tests, the model id, the tokens used and Token
Factory's request id.

For the squad's automatic reviews, the **daemon** needs the key in *its*
environment. A daemon started by a login service (`equipe daemon install`) does not see
your shell's variables, so run it in a terminal instead:

```bash
equipe daemon stop
read -rs NEBIUS_API_KEY && export NEBIUS_API_KEY
equipe daemon run               # in the foreground; Ctrl-C stops it
```

Without a key nothing is sent anywhere: the daemon logs once that reviews are off.
(For people without a key of their own, `equipe judge-relay` can hold one on a server and hand out
capped app tokens — see `docs/hackathon/judge-access.md`; it isn't deployed anywhere.)

| Variable | |
|---|---|
| `NEBIUS_API_KEY` | Your Token Factory API key. Read from the environment only — never written to disk, logged, or shown (`Debug` redacts it; error messages never include it or the auth header). |
| `EQUIPE_NEBIUS_MODEL` | A model id to use. Unset: equipe lists `GET /models` and picks an NVIDIA Nemotron chat model (`nvidia/…` with `nemotron` in the id; Super, then Ultra, then Nano). No such model → a clear error, no fallback to another vendor's. |
| `EQUIPE_NEBIUS_BASE_URL` | Another server with the same API (default `https://api.tokenfactory.nebius.com/v1/`). The tests point it at a local mock. |
| `EQUIPE_NEBIUS_MAX_TOKENS` | The longest answer asked for (default 2048). |

`.env.example` lists them; equipe itself doesn't read `.env` files.

**Expected cost per review — an estimate, not a measurement.** No review has been
measured against the real API yet. The diff is cut at 48,000 characters (≈12k tokens
at ~4 characters per token) and the ticket at 6,000; with the instructions, the input
is at most ≈14k tokens and usually far less (a 200-line change is ≈2–3k). The answer
is capped at 2,048 tokens and is usually a few hundred. Cost ≈ input tokens × the
model's input price + output tokens × its output price — look the prices up for the
model you use on Token Factory's pricing page. With *hypothetical* prices of $0.10 per
million input tokens and $0.40 per million output tokens, the largest possible review
would be about 14,000 × 0.10/10⁶ + 2,048 × 0.40/10⁶ ≈ $0.0022. Each automatic review
also makes one `GET /models` call unless `EQUIPE_NEBIUS_MODEL` is set.

**Security notes.**
- The diff and the ticket are sent to Nebius Token Factory. Don't enable it on code
  you may not send to a third party; without `NEBIUS_API_KEY` nothing is sent.
- The prompt tells the model that the diff and ticket are data, never instructions,
  and fences them. What comes back is treated as untrusted: thinking blocks dropped,
  only the first JSON object read, control characters and terminal escape sequences
  removed, every field cut to a short length, at most five risks and five tests.
  An answer that isn't a review is an error — a review is never made up.
- The daemon's log records the model id, Token Factory's request id, token counts and
  latency — never the key, the diff or the review's text.
- Requests time out after 90 s (10 s to connect); a rate limit (429), a server error or
  a timeout is tried again at most twice, with backoff (a `Retry-After` of up to 10 s is
  honoured); 400/401/403 are not retried.

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
