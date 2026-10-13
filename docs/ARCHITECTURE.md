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
| `equipe-core` (`crates/core`) | The brain: the `runtime` that runs members on engines; squads, roles and the board's rules; the journal in SQLite; the daemon's socket; the MCP board tools; worktree sandboxes | protocol, providers |
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
| Role         | `equipe_core::roles` | charter, file access, model tier, engine preference |
| Rules        | `equipe_core::board` | assignment and order — code, never a model's word; budgets and gates next |
| Board        | `equipe_core::store` | tickets, events, agent logs in SQLite |

## Engines

| Kind | What runs a turn | Today |
|---|---|---|
| `llm` | A thin provider call — no tools, no loop | **Codex Responses** (`openai::responses`): one streamed turn on the ChatGPT backend. For roles that read and report (PM, QA, data) |
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

Each squad's board lives in its database (`~/.equipe/squads/<squad>/squad.db`,
`equipe_core::journal`): tickets, and an append-only record of what happened to
each — created, assigned, handed in, commented, accepted, reopened. Tickets are
kept forever (a few hundred bytes each); conversations follow the storage policy.

Status flow: `proposed → todo → in_progress → review → done` (or `cancelled`).

- **The PM writes the board** through tools; a ticket has an owner (a role or a
  member), a project when the squad has several, dependencies, and acceptance
  criteria. A ticket from any other member is a *proposal* until the PM or the
  person makes it `todo`.
- **equipe's rules hand out the work** (`equipe_core::board::assignments`, plain
  code): a `todo` ticket whose dependencies are finished goes to a free member who
  owns it, oldest first, one ticket per member. The rules run whenever something
  changes — a ticket, a member joining or finishing a turn.
- **A member works on its ticket** in the ticket's project: an editing member in a
  worktree of its own on `equipe/<name>-t<id>-…`, in a conversation of its own
  for that ticket; its desk conversation waits.
- **It hands the work in** (`submit_for_review`): at the end of the turn its
  changes are committed to the branch and the ticket goes to `review`. The
  worktree stays, so the work can be sent back.
- **Only the person closes work.** `/accept T-3` makes it `done` (the worktree
  goes, the branch stays); `/reopen T-3 <what to change>` gives it back to
  its member on the same branch and conversation; `/cancel T-3` drops it. These are
  commands from the person's screen straight to equipe — no member can send them.
- **Merging is the person's too** (principle 4). Right after `/accept` the screen
  asks "merge it into main now? y/n", then whether to push main to `origin`; only
  their yes sends `merge`. `/merge T-3` (or `m` on the board) does it later. The
  merge is worked out apart from any checkout (`git merge-tree`) and a conflict
  merges nothing; a checkout that has main out follows by fast-forward, keeping
  its uncommitted changes, or refuses — and then nothing moves. Work accepted but
  not on main says so on the board ("not on main").
- **Accepted work is where the next ticket starts**, merged or not: a new
  ticket's worktree is cut from `HEAD` with the branches of the `done` tickets it
  depends on (and theirs) merged in. When they conflict, it doesn't start: it goes
  back to a proposal saying which ticket and files, for the person or the PM.
  Both, not one: without the merge question work piles up off main (equipe-site,
  October 2026: eight accepted tickets, a README on main); without starting from
  dependencies, a person who wants to look before merging holds the squad up.

### Board tools (MCP)

A Claude Code member gets its tools from `equipe mcp`, a stdio MCP server equipe
starts for it (`--mcp-config`). The server holds nothing: each call goes to the
daemon over its socket (`tool` in the protocol), where the runtime decides. The
member's identity comes from how equipe started the server, not from anything the
model says.

| Tool | Who | Does |
|---|---|---|
| `board` | everyone | the open tickets as text |
| `create_ticket` | everyone | the PM's are `todo`; anyone else's are proposals |
| `update_ticket` | the PM | change a `proposed`/`todo` ticket; accept or drop a proposal |
| `comment` | everyone | add to a ticket's record |
| `submit_for_review` | the member on a ticket | hand it in |

`--strict-mcp-config` keeps the person's own MCP servers (mail, calendar…) away
from the squad, and `--allowedTools mcp__equipe` lets a headless run use equipe's
tools without a prompt. Codex members have no tools until Codex runs over ACP,
whose `session/new` carries MCP servers; until then a Codex member says when it is
done and the person accepts.

### Plans are notes; tickets are commitments

Engines report their own step-by-step plan (ACP `agent-plan`). It is **not** a
ticket, just as a developer's notebook is not the team's tracker:

1. A plan belongs to the ticket being worked and is shown beside it as progress
   (`2/4`). Each update replaces the previous plan whole (ACP requires this).
2. Plan entries never become tickets on their own.
3. Work outside the ticket's scope is **proposed** as a new ticket (through a board
   tool equipe offers the agent over MCP); the PM or the person accepts it (`create_ticket` from anyone but the PM is a
   proposal).
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

### The daemon

The runtime lives in a daemon, one per user: `equipe daemon run`, started in the
background by `equipe` when none is answering and detached from the terminal
(`setsid`), so closing the terminal never reaches it or the agents it runs.

- **The door** is a Unix socket, `~/.equipe/equipe.sock` (0600 in a 0700
  directory): no network port, and only its owner can drive the squad. Messages
  are the protocol in JSON-RPC 2.0, one per line — `hello` (attach to a project,
  get its snapshot), `command`, `event`, `shutdown`.
- **Attaching** is one step of the runtime loop: the snapshot and the
  subscription to everything after it are taken together, so no event is missed
  or doubled. A screen follows one project (its repository root); sessions opened
  there from any screen appear on all of them.
- **The journal** keeps one SQLite database per project under
  `~/.equipe/projects/<name>-<hash>/`: the session record (engine, model, branch,
  title, tokens, the engine's own conversation id) and the conversation — messages,
  finished replies, one line per tool use. Streaming fragments are joined before
  they are stored; raw material (file contents, command output, images, thinking,
  diffs) never is. Conversations past the retention window (90 days) are pruned
  daily and the space returned (incremental vacuum); the record stays.
- **Restarting** resumes every open session: Claude Code continues with
  `--resume`, Codex with its journaled history, editing sessions in their
  worktrees. Stopping interrupts running turns; it never closes sessions.
- **Builds**: a screen and the daemon compare build ids; a daemon left running
  from an older build is pointed out, not restarted under agents mid-turn
  (`equipe daemon restart`).
- `equipe try` stays a standalone, in-process runtime for scripts and CI.

Events fold into a conversation through one shared function
(`equipe_protocol::Transcript`), used by the journal and every screen alike, so a
screen that attaches later sees exactly what a screen that watched live saw.

### The team console

`equipe` with no command opens the terminal UI (`ratatui`), a client of the
daemon. It takes the whole screen (alternate screen) because a team means several
agents updating at once — something a scrolling transcript can't show. Leaving
(Ctrl-C twice, `/quit`) only detaches: the agents keep working, and a summary —
who is still working, branches landed — is printed to the normal screen.

- Left: the team — each agent, its state (`● working`, `○ ready`), its model or
  branch. Right: the selected agent's conversation and work. Bottom: the message.
- Pastes arrive whole (bracketed paste): a newline inside a paste never sends.
  A multi-line draft shows its size before it's sent.
- Enter sends (a memo, if the member is mid-turn) · Alt-Enter / Ctrl-J add a line ·
  Esc stops the member · Tab or `@name` picks who to talk to · Ctrl-B (`/board`)
  shows the board, the person's reviews first · PgUp/PgDn or the wheel scroll ·
  `/mouse` hands the mouse back to the terminal for selecting text · Ctrl-C clears
  the draft, twice leaves · `/add`, `/accept`, `/reopen`, `/cancel`, `/close`,
  `/help`, `/quit`.
- Replies are Markdown, drawn as such: bold, code, lists, code blocks, and tables
  whose cells wrap inside their columns — wrapped by the console, Hangul counted
  as two columns.
- Work waiting for the person is always in sight: `1 to review` in the status bar.
- Drawn only when something changed; nothing redraws on a timer.

## The squad

A **squad** is the person's team, not a repository: the way a company's squad
owns a product's app, API and site at once, one squad works on several projects
(`equipe_core::squads`). Everything it keeps lives in `~/.equipe/squads/<squad>/`;
each project belongs to one squad. A screen follows a squad, whichever of its
repositories it was opened in.

**Onboarding** is the first `equipe` in a repository, under a minute: engines are
checked (a squad needs one to run on), then the repository joins an existing squad
— most people have one — or starts a new one, which the person names and staffs
(the PM, plus designer and frontend ticked by default). Nothing is written into
the repository.

A **member** is a long-lived session with a role in a squad: a charter, an engine,
a model, its own memory, and — while on a ticket — that ticket's project and, for
roles that edit, a worktree. Between tickets it sits at its desk: it reads every
project of the squad (`--add-dir`) and writes nowhere. Roles live in
`equipe_core::roles`:

| Role | Does | Edits files | Model tier |
|---|---|---|---|
| `pm` (or `po`) | turns what the person wants into a plan and tickets; the main contact | no | top |
| `designer` | flows, layout, visual language, tokens | yes | upper |
| `frontend` | pages, components, styles, client logic | yes | upper |
| `backend` | servers, APIs, data models, integrations | yes | top |
| `qa` | reviews, test plans, edge cases, acceptance | no | mid |
| `data` | metrics, analysis, answers with numbers | no | mid |
| `custom <name>` | whatever its name says | no | upper |

- **Engines and models.** A tier is resolved against what the person's plan
  offers: Claude Code by alias (top `opus`, otherwise `sonnet`), Codex from its live
  catalog (its first model, or a `mini` one for the mid tier). Editing roles run on
  Claude Code for now; the others prefer Codex — which also puts QA on a different
  engine from the engineers, so a second model checks the first one's work.
- **Charters** are added to the engine's own instructions, never replacing them:
  `--append-system-prompt` for Claude Code, `instructions` for Codex. A charter names
  the member, its teammates and who keeps the plan, and is rebuilt every turn, so it
  follows the team as members come and go.
- **Names** are unique per team — `frontend`, `frontend-2` — and editing members
  work on `equipe/<name>-<id>` branches.

### Talking to the team

**The PM is the person's main contact**, the way a CEO mostly talks to the PM.
But the person can talk to anyone, the way a CEO can walk over to any desk — Tab or
`@name` in the console. Three rules keep that from splitting the team:

1. **Direct talk is never hidden.** What the person tells a member shows in the
   PM's conversation (`↪ You → backend: …`), and what was said both ways travels
   with the PM's next message — so the PM keeps up without spending a turn on every
   exchange.
2. **A busy member is left a memo.** A message sent mid-turn waits and is
   delivered when the turn ends (memos left in the meantime arrive together). Esc
   stops the turn, and a waiting memo goes in at once — a tap on the shoulder.
3. **Scope changes become tickets.** A member asked for something beyond its task
   says so and proposes a ticket instead of quietly widening its work.

Approvals never pass through the PM: equipe asks the person directly, because no
agent's word — the PM's included — can authorize a merge or a deploy.

### Permission modes

How much a member may do on its own while it works a ticket — the squad's
default, or one member's (`/mode`, chosen at onboarding). At its desk a member
always only reads, and a role that doesn't edit never writes, whatever the mode.

| Mode | A member on a ticket | Claude Code |
|---|---|---|
| **plan** | reads and plans; changes nothing | `dontAsk` |
| **ask me** | asks the person before every change and command | `manual`, prompts to equipe |
| **auto** (default) | edits and runs commands inside its worktree; asks for anything beyond | `acceptEdits` + sandbox, prompts to equipe |
| **full access** | anything the person's account can do, except what equipe blocks | `bypassPermissions` |

Two layers, as in Codex (sandbox × approval policy) and Claude Code (sandbox ×
permission modes), with the presets on top (`equipe_core::permissions`):

- **A fence the operating system enforces.** In auto, Claude Code's sandbox
  confines a member's shell commands: they write only inside its worktree and
  reach only package registries and source hosts (`ALLOWED_DOMAINS`), and run
  without asking while they stay inside. A prompt can talk a model into anything;
  it can't talk the kernel out of a sandbox.
- **Questions for everything beyond it**, put to the person. Claude Code sends its
  permission prompts to equipe's `approve` tool (`--permission-prompt-tool`); the
  console shows the exact command, who asks, for which ticket and why, in a fixed
  place above the composer — `y` allow, `a` always allow, `n` decline. Commands
  that reach outside — `git push`, publishing, deploying (`OUTWARD`) — ask even
  inside the fence. A command that must leave the fence asks too, and runs
  outside only when allowed.
- **"Always allow"** becomes a rule for the whole squad (`Bash(npm install:*)`,
  `WebFetch(domain:docs.rs)`), kept in its database and passed to every member's
  turn. A chained command (`a && b`) is allowed once, never as a pattern.
- **Never, in any mode** (`BLOCKED`): `sudo`, `rm -rf /`, force-pushes and the
  like are refused without asking — by the engine (`--disallowedTools`) and by
  equipe, which checks every command of a chain.
- **Nobody answering is a no.** A request waits 30 minutes for the person, then is
  declined; stopping the member withdraws it. Every answer is noted on the ticket's
  record.

### Memory

Claude Code keeps an auto memory per directory, shared with the person's own
sessions there — run naively, a member would read the person's private notes and
write into them. Each member instead keeps its own: equipe moves Claude Code's
auto memory to the member's directory with a setting of the unmodified binary
(`--settings '{"autoMemoryDirectory": …}'`), under
`~/.equipe/squads/<squad>/members/<name>/memory`. It belongs to the member, not a
project: like a person, a member carries what it learned from one project to the
next. What a project needs everyone to know belongs in the repository
(`CLAUDE.md`, `AGENTS.md`), which every member reads.

Members are told they run inside equipe without a screen, so they never ask the
person to press keys or change Claude Code's permission mode — equipe sets it.

## Desktop

`equipe-desktop` renders the same core in a native GPUI window. Its rules
("draw nothing when nothing changed", logs never in UI memory) and measurements are
in [DESKTOP.md](./DESKTOP.md); the stack research behind it is in
[research/desktop-stack](./research/desktop-stack/README.md).
