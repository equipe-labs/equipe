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
| `equipe-desktop-control` (`crates/desktop-control`) | Members working the Mac's desktop apps: the tools and their aiming checks, the helper's socket, installing and signing the helper, the macOS calls (ScreenCaptureKit, CGEvent, Accessibility) | protocol |
| `equipe` (`crates/cli`) | Screens: the command line and the terminal UI | protocol, core, desktop-control |
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

Status flow: `proposed → todo → in_progress → review → done` (or `cancelled`),
with a side loop `in_progress → waiting → todo → in_progress` for a ticket that
can't go on without the person.

| Status | Means | Who moves it on |
|---|---|---|
| `proposed` | a member's suggestion | the PM or the person makes it `todo`, or drops it |
| `todo` | ready once its dependencies are done | equipe's rules hand it to a free owner |
| `in_progress` | a member is on it, in its worktree | the member hands it in or puts it down |
| `waiting` | put down until the person does their part; the note is their checklist | the person (`/resume T-3`) or the PM (`update_ticket` → `todo`) |
| `review` | handed in | the person accepts, reopens or cancels |
| `done` / `cancelled` | closed | the person can reopen |

- **The PM writes the board** through tools; a ticket has an owner (a role or a
  member), a kind (`code`, `task`, `action` — see *Ticket kinds*), a project for
  `code`, dependencies, and acceptance criteria. A ticket from any other member is a *proposal* until the PM or the
  person makes it `todo`.
- **equipe's rules hand out the work** (`equipe_core::board::assignments`, plain
  code): a `todo` ticket whose dependencies are finished goes to a free member who
  owns it, oldest first, one ticket per member. The rules run whenever something
  changes — a ticket, a member joining or finishing a turn.
- **A member works on its ticket** in a worktree of its own on
  `equipe/<name>-t<id>-…` — of the ticket's project for `code`, of the squad home
  for `task` and `action` — in a conversation of its own for that ticket; its
  desk conversation waits.
- **It hands the work in** (`submit_for_review`): at the end of the turn its
  changes are committed to the branch and the ticket goes to `review`. The
  worktree stays, so the work can be sent back.
- **It puts the ticket down when it needs the person** (`block_on_person`): a
  token to issue, an account or a setting to change. At the end of the turn its
  changes so far are committed, the ticket goes to `waiting` with the person's
  checklist (`- [ ] …`) as its note, and the member is free — the rules hand it
  its next ticket. The worktree, the branch and the ticket's conversation stay.
  The board and the card under every conversation show waiting tickets, and
  what each needs, beside the reviews. When the person has done it,
  `/resume T-3 <what I did>` (or the PM setting it back to `todo`) returns it to
  the member that put it down — while it is still on the squad — in the same
  worktree and conversation. A restart never picks a `waiting` ticket back up:
  only `in_progress` tickets resume on their own.
- **Only the person closes work.** `/accept T-3` makes it `done` (the worktree
  goes, the branch stays); `/reopen T-3 <what to change>` gives it back to
  its member on the same branch and conversation; `/cancel T-3` drops it. These are
  commands from the person's screen straight to equipe — no member can send them.
- **Merging is the person's too** (principle 4), for `code`; a `task` merges
  into the squad home as part of `/accept` (see *Ticket kinds*). Right after `/accept` the screen
  asks "merge it into main now? y/n", then whether to push main to `origin`; only
  their yes sends `merge`. `/merge T-3` (or `m` on the board) does it later. The
  merge is worked out apart from any checkout (`git merge-tree`) and a conflict
  merges nothing; a checkout that has main out follows by fast-forward, keeping
  its uncommitted changes, or refuses — and then nothing moves. Work accepted but
  not on main waits in the inbox ("Accepted, not in main yet") with why, when
  equipe knows (`off_main` on the ticket: a conflict, a failed merge, or kept on
  its branch by the person's `keep_on_branch` — which takes it out of the inbox
  for good; a later `merge` still lands it). A branch with nothing left to bring
  to main — in it already, or `+0 −0` because its changes landed another way or
  cancel out — counts as merged: at acceptance, after any merge, and on the
  daemon's minute.
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
| `update_ticket` | the PM | change a `proposed`/`todo`/`waiting` ticket; accept or drop a proposal; resume a waiting one |
| `comment` | everyone | add to a ticket's record |
| `submit_for_review` | the member on a ticket | hand it in |
| `block_on_person` | the member on a ticket | put it down until the person does their part |

`--strict-mcp-config` keeps the person's own MCP servers (mail, calendar…) away
from the squad, and `--allowedTools mcp__equipe` lets a headless run use equipe's
tools without a prompt. Codex members have no tools until Codex runs over ACP,
whose `session/new` carries MCP servers; until then a Codex member says when it is
done and the person accepts.

### Desktop tools (experimental, macOS)

Members can work the Mac's desktop apps the way they work the browser — the
design and what the T-20 spike proved are in [DESKTOP-CONTROL.md](./DESKTOP-CONTROL.md).
Off for everyone until the person runs `equipe desktop on <member>
--experimental`; until the screen lease (T-22) and the send approval (T-23)
exist, it stays behind that flag.

```
member's claude ──stdio──▶ equipe mcp --server desktop ──socket: desktop──▶ daemon
                                                     (is this member allowed?)
daemon ──~/.equipe/desktop/helper.sock + token──▶ equipe Desktop Helper.app
                                                   (LaunchAgent im.equipe.desktop-helper, Aqua)
```

- **Only the helper holds the permissions.** A TCC grant belongs to the process
  launchd started and every child inherits it: granted to the daemon, every
  member's Bash could capture the screen. The helper is an app bundle
  (`im.equipe.desktop-helper`) whose executable is a copy of `equipe` that runs
  `desktop serve` and nothing else, signed with a fixed identity so a rebuilt
  helper keeps its grants: a self-signed certificate made on the Mac, in a
  keychain of its own under `~/.equipe/desktop/signing`, until there is a
  Developer ID (`equipe desktop setup --sign "<identity>"` switches; `Signer` in
  `install.rs`). The bundle lives in `~/Applications` — System Settings lists
  only apps Launch Services knows, not bundles in hidden folders like
  `~/.equipe` — and a new build is staged beside it and swapped in, since App
  Management blocks rewriting a bundle in place.
- **Reached through the daemon.** The helper answers only requests carrying the
  token beside its socket, which fenced members can't read (`deny_read`, like the
  credentials). The runtime checks the member is turned on; the call itself runs
  outside the runtime's loop. The helper serves one request at a time.
- **Aimed, not just placed.** Coordinates are pixels of the member's last
  screenshot (captured at the model's size, ≤1568 px and ≈1.15 MP); the helper
  maps them to points. Before a click it brings the app forward (AX
  `AXFrontmost`/`AXRaise`) and asks Accessibility's hit test what the point
  reaches — the window list lies under BetterDisplay's overlay — and refuses if
  it isn't the app the member means. Typing and keys are refused unless the
  keyboard focus (`AXFocusedApplication`) is in that app, and typing stops the
  moment it moves. An element from `elements` is pressed through Accessibility
  where it can be. These rules are tested against a fake desktop
  (`controller.rs`).
- **No images kept.** A screenshot is a JPEG in memory, passed back as the tool
  result; the journal records the tool call's one line. (Claude Code's own
  transcript keeps tool-result images — equipe can't stop that.)
- `equipe desktop setup` installs the helper, asks macOS for the permissions,
  opens the two Settings panes and checks: a GUI session, the daemon as a login
  service there, Screen Recording, Accessibility (which takes the person's
  password or Touch ID to switch on), sending input, the display. `equipe
  desktop status` checks without changing anything; members without the
  permissions are told to ask the person for `equipe desktop setup`.

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

The squad home is a repository like any other to this code: `task` and `action`
tickets get worktrees of it under `~/.equipe/worktrees/<squad>.home/` (named by
squad, since every home's folder is called `home`), cut and merged by the same
functions.

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
  are the protocol in JSON-RPC 2.0, one per line — `hello` (attach to a squad,
  get its snapshot), `command`, `event`, `shutdown`, `version`, and for screens
  that can't read the daemon's disk `squads` and `upload` (`equipe_protocol::rpc`).
- **From another machine** a screen reaches the door over SSH:
  `ssh <host> equipe attach --stdio` carries it on standard input and output,
  after one `attached` line saying which builds answered. SSH signs in; equipe
  adds no port and no credential. See [DESKTOP.md, "Remote"](./DESKTOP.md#remote-a-daemon-over-ssh).
- **Attaching** is one step of the runtime loop: the snapshot and the
  subscription to everything after it are taken together, so no event is missed
  or doubled. A screen follows one squad, wherever it was opened; sessions opened
  in it from any screen appear on all of them.
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
each project belongs to one squad. **Work starts at the squad, not at a
repository** (decided 2026-10-13): `equipe` opens a squad wherever it is run, a
squad can exist with no project at all, and work that isn't code has a place of
its own — the squad home. The picture is an assistant like OpenClaw or Hermes,
reached from anywhere, that happens to be a team and happens to also write code.

```
~/.equipe/
  state.json                 last squad opened; repositories the person said no to
  squads/<squad>/
    squad.db                 board, record, projects, rules
    home/                    the squad home: an internal git repository (main)
    members/<name>/memory/   each member's own memory
  worktrees/<repo>/…         code tickets, one per ticket (as before)
  worktrees/<squad>.home/…   task and action tickets, one per ticket
```

### Where equipe starts

`equipe` with no command picks the squad to show, first match wins
(`onboard::squad_for`):

1. `equipe --squad <name>` — the one named.
2. The current directory is inside one of a squad's projects — that squad (as
   today).
3. It is inside a squad home or one of its worktrees — that squad.
4. The squad opened last (`state.json` → `last_squad`, written on every attach)
   — or, if that one is gone, the only squad, or the first by name.
5. No squad at all — onboarding.

No directory is special any more: the home folder, `/`, Downloads and a plain
folder are all simply "not a project", and the squad opens as in 4. Nothing is
asked there.

**A repository no squad has.** When the directory is inside a git repository that
belongs to no squad, the squad from step 4 opens and the console asks once, in
the approval slot above the composer: *"equipe-site isn't in a squad. Add it to
acme? y/n"* (with several squads, `y` adds to the one shown; `/squad add` puts it
elsewhere). `n` is remembered in `state.json` (`declined`, by repository root) and
never asked again for that repository; `/squad add .` still adds it. Only a git
repository is offered: a plain folder joins only when typed (`/squad add <folder>`).

**Onboarding without a repository**, under a minute, from anywhere: engines are
checked (a squad needs one to run on), then the person names the squad (suggested:
the git remote's owner when run inside a repository, else their user name, slugged
— `abel` → `abel`, taken → `abel-2`), staffs it (the PM, plus designer and
frontend ticked by default) and picks the mode. The squad and its home are created
with **no project**. If it was run inside a repository, one last question: *"Add
equipe-site to the squad? Y/n"*. Nothing is ever written into a repository.

**Switching**: `/squad <name>` (as today) re-attaches the console to another
squad and records it as the last; `/squad new <name> [folder]` starts one, the
folder now optional; `equipe squads` lists them with their homes. A squad with no
project is valid everywhere: `/squad remove` may take its last project away.

### The squad home

Every squad has a **home**, `~/.equipe/squads/<squad>/home`: a git repository
equipe keeps for it and never shows as one. It is where members are by default,
and where the squad's work that isn't code lives — research, notes, drafts,
reports, the things a task produces.

- **Made** with the squad (`squad_home::ensure`, idempotent): `git init -b main`,
  a local `user.name equipe` / `user.email equipe@localhost` (so it works without
  the person's git config, and its commits never carry their name), and a first
  commit with:

  ```
  README.md     what this folder is, in the person's language, for Finder
  notes/        research, findings, references — the squad's notebook
  outputs/      what tasks hand over: outputs/T-12 teams-reply/…
  .gitignore    .DS_Store, *.tmp
  ```

  (`notes/` and `outputs/` hold a `.gitkeep`, since git keeps no empty folder.)
  equipe's own git calls in the home pass `commit.gpgsign=false`,
  `core.hooksPath=/dev/null` and its own name, so the person's git setup never
  gets in the way.

  Folders other than these are fine; the charter tells members the layout, and
  equipe writes no `CLAUDE.md` there — one the person or the PM adds is theirs,
  and every member reads it, since the home is their working directory.
- **Members write it in conversation** (decided 2026-10-13, changing T-25's
  "desk never writes"): the person wants work that isn't development to just
  happen, so a short thing asked at the desk — save a note, tidy material, make
  one table — is written straight into the home, with no ticket. Each change is
  **saved as soon as the turn ends** (`squad_home::save`), one line saying who and
  why: `pm: 세금 마감일 메모해줘` — the member's name and the person's words. The
  member's conversation shows *"✎ pm saved notes/tax-deadline.md in the squad's
  folder · /home undo takes it back"*.
- **Every change can be taken back.** `/home changes` (and `equipe home changes`)
  lists the latest ten — when, who, which files, why — numbered; `/home undo [n]`
  takes back the latest or the n-th (`git revert`, saved as a change of its own by
  the person, so an undo can be undone). A change that later ones built on is left
  as it is, and the person told why.
- **The person may open it.** `/home` in the console (and `equipe home`) opens it
  in Finder (`open`, `xdg-open`) and prints its path; outputs are named for people
  to find. The person may edit or drop files there. Before equipe saves, takes
  back or merges anything in the home, it saves whatever the person changed as
  theirs (`the person: changed in the squad's folder`) — before each desk turn,
  when no member is mid-turn at its desk — so nothing they did is lost or
  credited to a member, and they never need git.
- **No git words reach the person.** Branches, merges and commits of the home
  appear in the console as "T-12's work is in the squad's folder", never as a
  branch name; the board shows no "not on main" for it.
- **Storage.** The home is the person's data, like a repository: never pruned by
  the retention window, not counted against the journal's size cap, listed with
  its size in `equipe storage`. It has no remote; backing it up is the person's
  (Time Machine covers it). Adding a remote later is an outward step that asks.
  Removing a squad keeps its home unless the person confirms deleting it.
- **Other squads' homes are private.** A member's fenced commands can't read
  another squad's home (`deny_read`, beside credentials and the browser), its
  file tools are denied it (`Read(//<home>/**)`, `Edit(//<home>/**)` deny rules),
  and a request through `approve` that names it is refused without asking.

### Where a member works

| Where | Working directory (`cwd`) | Read too (`--add-dir`) | Writes |
|---|---|---|---|
| At its desk | the squad home (main) | every project | the squad home, each change saved |
| On a `code` ticket | the project's worktree | the squad home | the worktree |
| On a `task` / `action` ticket | a worktree of the squad home | every project | the worktree |

- **At the desk, by mode**: read-only writes nothing; ask puts each change to the
  person; auto writes the home on its own (`acceptEdits`), fenced in by the
  sandbox like a ticket; full writes freely. Repositories are read in every mode
  — code changes only on a `code` ticket, in its worktree.
- **What stays a ticket**: work of several steps, work handed to a teammate, and
  anything whose result the person should review before it counts (T-28's
  `task`). Anything reaching outside — sending, submitting, paying — asks the
  person first, at the desk or on a ticket. The charter tells members all this.
- A directory added for reading is **for reading only** in every access, bypass
  included: Claude Code would otherwise accept edits there too, so each one gets
  deny rules (`Edit(//<dir>/**)`, `Write(//<dir>/**)`, in the turn's
  `permissions.deny`), the sandbox's `denyWrite`, and equipe refuses through
  `approve` an edit — or, at the desk, a shell command that doesn't only look —
  naming it, without asking the person.
- A project's `CLAUDE.md` is loaded from an added directory only when
  `CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1` is set: equipe sets it for
  members (adding a variable, not removing the person's), so the desk still knows
  each project's conventions.
- **A folder the squad doesn't have** is asked for: a read outside the `cwd` and
  the added directories goes to the person through `approve` ("backend wants to
  read ~/Documents/plan.pdf"); `a` allows that folder for the squad
  (`Read(//<folder>/**)`), never write.

### Ticket kinds

A ticket has a **kind**, which says where its work happens and what the person's
approval does (`Ticket.kind`, `equipe_protocol::TicketKind`):

| Kind | Work happens in | Hand-in | `/accept` |
|---|---|---|---|
| `code` | a worktree of the ticket's project, on `equipe/<name>-t<id>-…` | a branch | `done`; then the merge question (as today) |
| `task` | a worktree of the squad home (`worktrees/<squad>.home/`), same branch naming | files under `outputs/` or `notes/` | merges into the home's main at once, then `done` — no question, no git words |
| `action` | as `task`, plus doing something in an app or the browser | what was done, and its notes | as `task`; each outward step was approved before it happened (T-23's gate) |

- **Choosing the kind.** `create_ticket` and `update_ticket` take `kind`. Left
  out, it is `code` when the ticket has a project — named, or the squad's only
  one — and `task` otherwise, so a ticket with no project is never stuck (a squad
  with one project keeps today's behaviour). `code` needs a project: named, or
  the squad's only one; with several and none named, the tool answers "name the
  project", as today. A `task` or `action` may name a project as what it is
  *about* (the console files it there), but never writes to it. A ticket that has
  work under way (a branch) keeps its kind. The PM's charter says: changing a
  repository → `code`; finding out, writing, deciding → `task`; doing something
  in the world → `action`.
- **Shown** on the board as text for models (`T-4 [todo] … — task, owner data`),
  and in the console as a tag before the title (`task · Competitor research`) and
  in the ticket's facts. A task's or action's ticket shows no branch and no
  "not on main"; `d` shows its files against the home's main.
- **Assignment** (`board::assignments`): a `code` ticket is ready when it knows its
  project (or the squad has one); a `task` or `action` ticket needs no project.
- **Who may write.** `Role.edits` is about repositories: on a `task` or `action`,
  every role writes in its home worktree (a data member writes its report, the PM
  its plan), within the mode — plan still changes nothing.
- **Hand-in.** A task's review note is the member's summary followed by the files
  its branch leaves in the folder (`In the squad's folder: - outputs/T-4 …/report.md`);
  an action's adds what the person allowed or declined on the way, from the
  ticket's record (`What you approved on the way: - ✓ …`). The work prompt tells an
  action's member to wait for the person's yes before anything outward and to
  hand in a step-by-step log as its summary.
- **Approval of a task** (`Runtime::land_in_home`). equipe saves the person's own
  changes in the home (when no member is mid-turn at its desk), then merges the
  branch into main (`worktree::merge_into_main`, as for code), the commit saying
  who and which ticket (`data: T-4 Competitor research`, so `/home changes` lists
  it). A conflict merges nothing and closes nothing: the ticket goes back to its
  member (`todo` for that member, then `in_progress` in the same worktree and
  conversation) with the note "the squad's folder changed under this work, so it
  couldn't go in…" — the member merges main into its branch inside its fence.
  Only a clean merge makes it `done`. Worktree and branch then go. `/merge` of a
  task says its work is in the folder already; the console asks no merge
  question for one. The home's own settings turn off commit signing and hooks
  (`squad_home::ensure`), since its worktrees commit with plain git.
- **Dependencies** work across kinds: a `task` cut after a done `task` starts from
  the home with that work in (it is on main already); a `code` ticket after a
  `task` reads the outcome in the home.

**Moving existing squads.** On the first start of a daemon that knows the home,
each squad without `home/.git` gets one (`squad_home::ensure`, in `restore`).
Projects, tickets, branches and worktrees stay as they are. Existing tickets get a
kind: `code` if they have a project, `task` if not (`kind TEXT NOT NULL DEFAULT
'code'` plus that backfill). A member's desk conversation was opened in a project,
and Claude Code resumes a conversation only from the directory it began in, so
each desk starts afresh in the home from a summary (`refresh_desk`), once; ticket
conversations continue in their worktrees untouched.

### Members and roles

A **member** is a long-lived session with a role in a squad: a charter, an engine,
a model, its own memory, and — while on a ticket — that ticket's worktree (see
*Where a member works*). Between tickets it sits at its desk, in the squad home —
writing there, reading every project. Roles live in `equipe_core::roles`:

| Role | Does | Edits files | Model tier |
|---|---|---|---|
| `pm` (or `po`) | turns what the person wants into a plan and tickets; the main contact | no | top |
| `designer` | flows, layout, visual language, tokens | yes | upper |
| `frontend` | pages, components, styles, client logic | yes | upper |
| `backend` | servers, APIs, data models, integrations | yes | top |
| `qa` | reviews, test plans, edge cases, acceptance | no | mid |
| `data` | metrics, analysis, answers with numbers | no | mid |
| `custom <name>` | whatever its name says | no — yes with `--edits` | upper |

*Edits files* is about a project's files, on a code ticket. Every role writes the
squad home — at its desk, and on a `task` or `action` ticket in a worktree of it —
so research, writing and paperwork need no editing role. The PM does short work
itself in conversation, puts bigger work on the board, and when no member fits
owns it or suggests `/add`.

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
writes only the squad home, as the mode allows (see *Where a member works*), and
a role that doesn't edit never writes a repository, whatever the mode.

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
- **The fence is the same in the squad home.** A `task` or `action` member in Auto
  writes only inside its worktree of the home — not the home's main, not a
  project; the directories added for reading stay read-only (deny rules), and a
  folder the squad doesn't have is a question, not a read.
- **"Always allow"** becomes a rule for the whole squad (`Bash(npm install:*)`,
  `WebFetch(domain:docs.rs)`), kept in its database and passed to every member's
  turn. A chained command (`a && b`) is allowed once, never as a pattern.
- **Never, in any mode** (`BLOCKED`): `sudo`, `rm -rf /`, force-pushes and the
  like are refused without asking — by the engine (`--disallowedTools`) and by
  equipe, which checks every command of a chain.
- **Nobody answering is a no.** A request waits 30 minutes for the person, then is
  declined; stopping the member withdraws it. Every answer is noted on the ticket's
  record.
- **Sending in the person's name from a desktop app** (`equipe_core::desktop_gate`).
  Before the desktop helper makes an input, the daemon asks `Handle::desktop_gate`.
  In one of the squad's messaging apps (`/send-apps`; Teams, Slack, Mail… by
  default) whatever may send — Return, a click on a button that isn't plainly
  harmless, a drag — waits for the person, in every mode, with the draft, the
  conversation and a screenshot to open (`o`); `a` is never offered. Typing a
  draft, looking and scrolling go ahead; other apps never ask. Payment, account
  deletion, password fields and security settings take no input at all. See
  [DESKTOP-CONTROL.md](./DESKTOP-CONTROL.md#send-gate-t-23).

### The Mac's screen

One mouse, one keyboard: the daemon lets one member at a time use the desktop
tools, and the person always comes first (`equipe_core::desktop`). Every desktop
tool call asks the daemon before it reaches the Mac (`Handle::desktop_ask`) and
says when it has ended (`desktop_done`); the answer is code, not the model's.

- **A lease, across squads.** The first call gets the screen; the others are told
  "frontend is using the screen; you are number 1 in line" and keep their place.
  It passes to the next in line when the member's turn ends, when it lets go
  (`desktop_release`), or after 2 minutes without a desktop call. Looking
  (screenshots, the accessibility tree) waits its turn too: the one who has the
  screen may be halfway through something.
- **The person first.** macOS tells how long ago the last input of any source came
  (`CGEventSourceSecondsSinceLastEventType`). The members' own clicks count there
  too, so equipe tells them apart by *when* they ran: an event outside every
  window in which a member's call was posting is the person's — at the Mac, or
  over Jump Desktop, whose input is synthetic as well. After the person's last
  touch, members' input waits 30 seconds; looking stays allowed.
- **Locked or logged out**, no input (`CGSSessionScreenIsLocked`); members can't
  unlock it. Not knowing — a daemon outside the GUI session — counts as no.
- **`/desktop stop`** takes the screen back from everyone at once and empties the
  line; nothing is let through until `/desktop resume`. The console's header and
  `equipe status` show who has the screen and how many wait.
- Kept in memory only: a restarted daemon starts with a free screen.

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

**The squad's memory** (decided 2026-10-13, T-32). Work that has no repository —
an assistant's errands, taxes, research — had nowhere to keep what everyone
should know, so there are two layers:

| Layer | Where | Holds |
|---|---|---|
| A member's | `members/<name>/memory/` (above) | how it works: its own knack and lessons |
| The squad's | `home/memory/` (`equipe_core::squad_memory`) | what everyone needs: facts about the person (preferences, schedule, the services they use), decisions, recurring deadlines, terms, contacts |

- **One fact per file**, `memory/<name>.md`, with a frontmatter: `name`,
  `description` (the one line the index shows), `kind` (`person`, `decision`,
  `deadline`, `term`, `contact`, `other`), `updated` (a date) and `by`. A file
  without one — dropped in by the person — is a fact too, named for its file and
  described by its first line.
- **equipe builds the index, not the members.** `memory/INDEX.md` is made from the
  files' frontmatter, so it can't drift from them: a file the person adds, fixes
  or deletes is in — or out of — the index on the next turn.
- **Reading: in the charter, every turn.** Each member's charter carries the index
  (up to 150 lines; past that, a pointer to `INDEX.md`) and the memory's path; a
  fact's body is read when it bears on the work — from the home at the desk, as an
  added directory on a ticket. The charter is rebuilt every turn, so a member
  started later, a resumed one and a Codex member (no file tools) all know the
  same. Not `home/CLAUDE.md`: equipe writes none there (the person's or the PM's
  is theirs), and a charter reaches every engine and every place a member works.
- **Writing: equipe's `remember` and `forget` tools**, from the desk and from a
  ticket alike — on a ticket the home is only read, and a tool is how a member
  writes past that fence without opening it. `remember` takes a name, the line, a
  kind and an optional body; the same name updates the fact (so a duplicate is an
  update), and `forget` removes one that turned out wrong. Each is saved in the
  squad's folder at once, by the member (`pm: remembered tax-filing: …`), shown to
  the person (*"✎ pm remembered for the squad: … · /memory shows what the squad
  knows"*), and taken back with `/home undo` like any change there. Members are
  told to write facts only through the tools.
- **What the person wrote wins.** Before a tool writes, whatever the person changed
  in `memory/` is saved as theirs; a fact whose latest change is theirs isn't
  overwritten until the member asked them and calls again with `confirmed`.
- **The person sees and fixes it.** `/memory` (and `equipe memory`) opens the
  folder in Finder and lists the facts and the latest changes; `/memory changes`
  lists the last ten changes to the memory alone.
- **Tidying.** Past 60 facts (`CROWDED`, a few screens of index) the PM's charter
  asks it to propose a ticket that merges overlapping facts and drops stale ones.
- **No secret values** (with T-31): the charter says to remember where an original
  is (`outputs/T-12 …/id.pdf`), never a resident registration, card or account
  number or a password, and `remember` refuses what looks like one (a registration
  number, a Luhn-valid card, an account-shaped run of digits; phone numbers and
  dates pass).
- **Private to its squad**: the memory is inside the home, which other squads'
  members can neither read nor be told about (see *The squad home*).

Members are told they run inside equipe without a screen, so they never ask the
person to press keys or change Claude Code's permission mode — equipe sets it.

## Desktop

`equipe-desktop` renders the same core in a native GPUI window. Its rules
("draw nothing when nothing changed", logs never in UI memory) and measurements are
in [DESKTOP.md](./DESKTOP.md); the stack research behind it is in
[research/desktop-stack](./research/desktop-stack/README.md).
