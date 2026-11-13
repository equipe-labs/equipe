# Using equipe

The commands and console features in detail. For a first look, start with the
[README](../README.md); for how it is built, [ARCHITECTURE.md](ARCHITECTURE.md).

## Commands

| Command | What it does |
|---|---|
| `equipe` | **The squad console**, from any folder. The first time it sets up your squad in under a minute — no repository needed; after that it opens the squad of the project you're in, else the one you used last (`--squad <name>` picks one), and in a repository no squad has it asks once whether to add it (`n` is remembered). Then: **one conversation with the PM, and the work as tickets.** On the left, the PM, then every ticket by where it stands — waiting for you, in progress, to do, done — then the team; on the right, what's open. Tell the PM what you want — it answers what's quick and puts the rest on tickets (its own work too, so it's never out of reach), equipe hands each to its owner, engineers work on their own branches and hand the work in. Open a ticket (Tab, or a click) to see its work and steer it as it goes; `/accept` or `/reopen` it (Ctrl-B shows the board). `@name` talks to someone directly — the PM hears about it. In **auto** mode members install, build and test inside a sandbox around their worktree; anything beyond — a push, a deploy — waits for your answer: `y` (or `a`) allows it once, `A` always, `n` declines (`/mode` switches between read-only, ask me, auto and full access). **The squad lives in a background daemon** — leave, or close the terminal, and it keeps working; run `equipe` again to come back |
| `equipe browser` | **One Chrome for the whole squad, signed in once.** equipe runs a Google Chrome of its own on this Mac — an ordinary one as sites see it — and `equipe browser` opens a window in it where **you** sign in, or sign up, wherever the squad should work: Google, GitHub, Vercel, PostHog… Once is enough: every member works in that Chrome, each conversation in a window of its own that sees only its own tabs and the popups it opened, signed in as you — no copies of your sign-ins, nothing to repeat per member. Members' windows open behind what you're doing and work minimized; each glows in the member's colour with its name at the foot, and outlines what it clicks or types into. They ask you before creating accounts, accepting terms, paying or deleting anything, and leave passwords, codes and CAPTCHAs to you. Needs Node.js and Google Chrome. In the console, Ctrl-W (or `/browser watch`) shows the member you talk to's window live above its conversation; `/browser` brings that member's window to the front; `/browser signin` opens your window. Code a member runs in its browser's own process asks you first in auto, showing the code |
| `equipe desktop setup` | **Experimental, macOS.** Members work this Mac's desktop apps the way they work the browser — screenshot, read an app's buttons and fields, click, type (any language, IME input included), shortcuts, open and bring apps forward. You give Accessibility (and, for screenshots, Screen Recording) to one entry, *equipe* — equipe.app itself, or without the app a small helper bundle named equipe; members never get them. The daemon installs the engine and sets the helper up by itself and asks only for the one Accessibility switch (a card in the app, or the first-run step); `setup` does the same right away and says what is still missing. Once the switch is on, every member works the desktop; `equipe desktop off <member>` (or `off --everyone` for the squad) takes it back, `on` gives it back, `locked on` lets members act through the accessibility tree while the screen is locked (an unattended Mac), `status` checks, `remove` uninstalls. Each click is checked against the app the member means, typing is refused when the keyboard focus is elsewhere and stops if it moves — leave the Mac alone while a member works it. Screenshots are never saved by equipe (Claude Code's own transcript keeps them). See [DESKTOP-CONTROL.md](DESKTOP-CONTROL.md) |
| `equipe daemon status\|stop\|restart` | The daemon itself. `equipe` starts it when needed; stopping it keeps open sessions, which resume when it starts again. `restart --when-idle` waits until no member is mid-turn — what to run after `equipe update`. `status` says who is working |
| `equipe daemon install\|uninstall` | Run the daemon as a login service (launchd on macOS, systemd on Linux): it starts when you log in, comes back after a crash, and outlives the terminal or SSH session that started it |
| `/add` | Teammates are yours to add: `/add <role…>` brings one in, `/add custom <name>` one of your own making, and the PM proposes one when a role's work keeps piling up — equipe never adds members on its own. Big work runs in parallel inside a member instead: it splits a ticket that divides cleanly across subagents and runs them together, each shown under its name |
| `/send-apps` | Where a member on the desktop may send something in your name — Microsoft Teams, Slack, Mail, Outlook, Messages, Zoom, Discord… A message is typed as a draft, but Return or the Send button waits for your yes every time, in every mode, showing the draft, the conversation and the screen. Payment, account deletion, passwords and security settings take no input at all. `/send-apps add <app>`, `remove <app>`, `reset` |
| `Ctrl-V` · drop | Ctrl-V attaches the clipboard's image (a screenshot) to your message; dropping an image file on the terminal attaches it too. **Over SSH**, run `equipe paste-setup` once on the Mac you type on (or `curl -fsSL https://github.com/equipe-labs/equipe/releases/latest/download/paste-setup.sh \| sh` if equipe isn't installed there) and ⌃⇧V in iTerm2 types the clipboard's screenshot down the connection, where equipe attaches it (`--remove` undoes it). Without it, `equipe send-image <host>` sends the clipboard image from your Mac |
| `/find` · `/tidy` | **f** on the board (or `/find <words>`) narrows it as you type — titles, notes and T-numbers, done work too. `g` shows open work as a tree of what waits for what. Once a day the PM hears which unstarted tickets haven't moved in a week and proposes what to close, merge or keep — it never drops work itself; `/tidy` asks now |
| When it's done | A member starting anything longer than a couple of minutes says when it expects to be done — and, when it ships something, when that will be live. The console shows it beside the member (`done ~20:52 (12m)`). When a member hasn't said, equipe guesses from how long the same kind of work took before, and says it's a guess |
| `/suggest` | After a member answers you, your likely next message waits greyed in the empty composer, and **→** takes it; typing ignores it. Claude Code's small model guesses it from the end of the conversation alone. `/suggest off` stops it, `/suggest on` brings it back |
| `/model` | `/model <member> <model>` changes the model a member works on from its next turn — Claude Code's `opus` / `sonnet` / `haiku`, or any model its engine offers; its conversation carries on |
| `/usage` | The footer shows your Claude plan's five-hour and weekly windows; `/usage` adds when they start over, and each member's context, tokens processed and the share from cache |
| `/compact` · `/clear` | `/compact [member]` summarizes a member's conversation now; `/clear [member]` starts clean. Memory and the board carry over either way |
| `/squad` · `/project` | `/squad <name>` switches to another squad, `/squad new <name> [folder]` starts one, `/squad add <folder>` / `/squad remove <project>` change what a squad works on. `/project <name>` (or **Ctrl-P**) filters the board, the review card and new requests to one project; `/project all` takes the filter off |
| `equipe telegram setup` | **Your squads from Telegram.** Make a bot with @BotFather, paste its token, tap the link it shows: the bot is paired with your Telegram account and answers no one else. Requests for permission arrive with Allow / Decline (once — never "always" from a phone), work to review with Accept / Ask for changes. Runs in the daemon; `equipe telegram off` disconnects. Messages travel through Telegram's servers (encrypted to them, not end to end) |
| `equipe pair` · `equipe devices` | Pair a phone running the equipe app with this machine. Phone and machine reach each other through the equipe relay ([`relay/`](../relay/README.md)), every frame sealed between the two ends (HPKE): the relay passes envelopes it can't open, and nothing listens on your machine. `equipe devices remove <name>` cuts one off at once. Protocol: [remote-protocol.md](remote-protocol.md) |
| `equipe login` | Pick an engine and sign in (`codex` / `claude` / `nebius`, `--again`, `--no-browser`) |
| `equipe logout` | Forget equipe's Codex login (Claude Code keeps its own) |
| `equipe status` | Which engines are ready, on which plan, with which models |
| `equipe review` | An NVIDIA Nemotron model on Nebius Token Factory reviews this repository's changes — see [OPEN-MODELS.md](OPEN-MODELS.md#nemotron-review-on-nebius-token-factory) |
| `equipe roles` | The roles a squad can have, what each does and the model it gets |
| `equipe squads` | Your squads and the projects each works on |
| `equipe squad delete <name>` | Delete a squad (asked first, saying how much space it frees; `--yes` for scripts). Its members stop. What can be made again goes at once: the worktrees equipe made for it (unless another squad's ticket uses them; their branches stay), its build caches, and build output in its folder. The rest — board, history, members' memory, home — moves to `~/.equipe/deleted/<name>-<YYYYMMDD-HHMMSS>` and is kept 7 days; moving it back to `~/.equipe/squads/<name>` with the daemon stopped restores it. `--purge` deletes it for good, now. The projects' folders and repositories aren't touched. The desktop app does the same from the squad switcher and Settings › Squad & machine |
| `equipe lang [code\|auto]` · `/lang` | The language equipe speaks to you — en, de, es, fr, ja, ko, zh-Hans, zh-Hant. Nothing picked (`auto`): `LANGUAGE`/`LC_ALL`/`LC_MESSAGES`/`LANG` in a terminal, then the system's preferred languages, else English. `EQUIPE_LANG` overrides everything. Members write to you in the language you write in |
| `equipe telemetry on\|off\|status` | Anonymous usage events, off until you say yes — the whole list is in [TELEMETRY.md](TELEMETRY.md) |
| `equipe update` | Update equipe to the latest release |
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

Every engine runs on a plan or key **you** already have. equipe never pays for, resells or
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

**Open models through the Codex CLI — bring your key.** With neither plan, a provider key
(NVIDIA Nemotron on Nebius Token Factory, or any OpenAI-compatible server) and the free,
open-source Codex CLI run members on open models, billed per token to your key — see
[OPEN-MODELS.md](OPEN-MODELS.md).

## Project checks

When the PM accepts a code ticket, equipe can run the project's own checks on the
branch **merged with main** before anything lands. A member saying its tests pass is
never enough: the branch may be days behind main, and two changes that each pass alone
can break each other.

Declare the checks in `.equipe/checks.json` at the top of the repository, committed to
main like a CI workflow:

```json
{
  "commands": [
    "cargo test --workspace --locked",
    "cargo clippy --workspace --all-targets --locked -- -D warnings"
  ],
  "timeoutMinutes": 40
}
```

- **Where they run.** equipe works the merge out without touching main, checks that exact
  commit out in a temporary worktree (under `~/.equipe/checks/`, removed afterwards; the
  project's installed `node_modules` are cloned in copy-on-write where the lockfile
  matches), and runs each command there through the shell (`sh -c`; `cmd /C` on Windows),
  one after another, off the daemon's loop. `EQUIPE_CHECKS=1` is set; Rust builds go to a
  build folder of the squad's own (`~/.equipe/cache/build/<squad>/_checks`), kept between
  runs and trimmed like the members' ones.
- **Pass:** the ticket is accepted, and the very commit that was checked lands on main
  (pushed if the PM was asked to push). If main moved meanwhile, the checks run again on
  the new main (up to three times).
- **Fail or time out:** the ticket goes back to its member, on the same branch, with the
  failing command and the last lines of its output. Main is untouched. If the same command
  also fails on main alone, main is broken already: the work stays in review, nothing is
  merged, and the PM and the person hear that main needs fixing first.
- **The file is read from main as it stands**, never from the branch being checked, so a
  branch can't weaken the checks it is held to. A file that can't be read (bad JSON, an
  unknown key, no commands) holds the merge back rather than guessing.
- **No file, no checks.** equipe doesn't guess a project's test command: full suites can
  take many minutes, and only the project knows what "passing" means. Such a project is
  merged as before, and the acceptance the person sees says no project checks ran.
- `timeoutMinutes` covers all the commands together: 30 when left out, at most 180.

The gate applies to the PM's own acceptance (`accept_work`). Work the person accepts and
merges themselves, from the board or a card, is merged as they say.
