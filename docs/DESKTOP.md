# Equipe desktop

All-Rust desktop app: `crates/desktop` (UI on [GPUI Kit](https://github.com/longbridge/gpui-kit) 0.7.1,
which pins its GPUI snapshot, `gpui-pre` 0.3.8). It is a window onto the daemon —
a third screen beside the terminal console and the phone — not a second engine.

```bash
cargo run --release -p equipe-desktop
```

Why a Rust core and GPUI: [research/desktop-stack](./research/desktop-stack/README.md).
Screen design: the squad's `outputs/T-54 데스크톱 화면 설계/spec.md`.

## How it is built: a daemon client

```
 equipe-desktop                                   equipe daemon
 ┌───────────────────────┐   ~/.equipe/equipe.sock   ┌───────────┐
 │ views/ ← state::Model │ ◄───── JSON-RPC lines ──► │ members,  │
 │            ▲          │                           │ board,    │
 │  daemon::Link (thread)│                           │ sessions  │
 └───────────────────────┘                           └───────────┘
```

- **Connection.** (From another machine: see [Remote](#remote-a-daemon-over-ssh).) `daemon::Target::from_env` picks the socket (`$EQUIPE_HOME`, else
  `~/.equipe`, `equipe.sock`) and the squad (`$EQUIPE_SQUAD`, else the one opened
  last, else the first). `daemon::spawn` runs a `Link` on a thread of its own
  (small tokio runtime) that connects, follows, and when the daemon goes away waits
  and retries (250 ms doubling to 5 s) — so a daemon turned back on is picked up
  without a click. It reports only *changes*: a daemon that stays down is one
  `Down`, not one per retry.
- **Methods used** (the same door the terminal console uses, `equipe_protocol::rpc`):
  `hello {version, squad}` → `Hello {version, snapshot}` once per connection (the
  snapshot replaces whatever the window held), then `event` notifications
  (`Event`: `WorkStarted/WorkEnded`, `ToolUse`, `TextDelta`, `TurnEnded`,
  `ApprovalRequested/Resolved`, `TicketChanged`, `SessionStarted/Closed`, …).
  `command` (notification) is wired (`Link::send`) but no screen sends yet.
- **State.** `state::Model` folds the snapshot and events into the member list and
  the connection (`Connecting` / `Connected` / `Down`). A member "needs you" when it
  has a permission request open or owns a ticket put down for the person. Pure
  logic, unit-tested without a window. The squad list is read from the folder names
  under `<home>/squads` at start and on every attach.
- **Screens.** `src/views/{sidebar,chat,board}.rs`; `chat` and `board` are
  placeholders that later tickets fill in, each in its own file. `main.rs` holds the
  window, the daemon pump and the connected / banner / daemon-off bodies.
- **Drawing.** Updates that land within 100 ms are applied together; a frame is
  requested only if what is on screen differs (`Model::apply` returns whether it
  does). `EQUIPE_LOG_RENDER=1` prints a line per frame with the memory footprint.
  The daemon-off screen says to run `equipe`; the app does not start the daemon.

## Remote: a daemon over SSH

The desktop runs on the laptop; the daemon may run on another Mac (a Mac mini)
that the laptop reaches with `ssh`. The app then speaks the same protocol, only
through SSH instead of the socket file.

```
 laptop                                       mini (the daemon's machine)
 equipe-desktop ── ssh -T mini '… equipe attach --stdio' ──► equipe attach
   stdin/stdout  ◄──────── JSON-RPC lines, untouched ───────►  ~/.equipe/equipe.sock
```

### Connecting

- **Target.** `daemon::Target` grows a transport: `Local { socket }` (today) or
  `Ssh { host }`, `host` as `ssh` knows it (`mini`, `me@mini.tailnet`, an alias
  from `~/.ssh/config`). Same retry loop as the local `Link`: each attempt is a
  fresh `ssh` process; when it exits, wait (250 ms doubling to 5 s) and run it again.
- **The command.** The app runs, with no shell of its own in between:

  ```
  ssh -T -o BatchMode=yes -o ServerAliveInterval=15 -o ServerAliveCountMax=3 \
      -o ControlMaster=auto -o ControlPath=<app dir>/ssh-%C -o ControlPersist=60 \
      <host> 'PATH="$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:$PATH" exec equipe attach --stdio'
  ```

  - `BatchMode=yes`: ssh never asks a question on a terminal the app doesn't
    have. Keys come from the person's agent / keychain and `~/.ssh/config`; a
    host whose key isn't known yet fails (exit 255) and the app says to run
    `ssh <host>` once in a terminal.
  - The `PATH` prefix: a command run over SSH gets a non-login shell, which
    on macOS usually lacks Homebrew's and `install.sh`'s folders.
  - `ControlMaster`: the side commands below (`git diff`, `equipe home …`)
    ride the same SSH connection — no second sign-in, no extra latency.
    `<app dir>` must be short (Unix socket paths ≤ ~100 bytes).
- **The first line** is a notification the bridge writes before anything the
  daemon says (`rpc::Attached`):

  ```json
  {"jsonrpc":"2.0","method":"attached","params":{"bridge":"0.21.0+1760400000","daemon":"0.21.0+1760400000","protocol":1,"started":false}}
  ```

  Then the app sends `hello {version, squad}` exactly as it does locally. The
  app compares `protocol` with its own `rpc::PROTOCOL`:
  equal → carry on; daemon's lower → "update equipe on <host>" (`equipe update`,
  then `equipe daemon restart`); daemon's higher → "update this app". `daemon`
  is `null` when that daemon predates the `version` method (treat as protocol 1,
  suggest `equipe daemon restart`). `bridge` ≠ `daemon` means the daemon runs an
  older build than the installed `equipe` — the console's "restart when idle"
  hint (`equipe daemon restart --when-idle`), not an error.
- **Daemon off.** The bridge starts it, as the console does: through the login
  service when `equipe daemon install` set one up, else detached (`setsid`),
  and says `"started": true`. Over SSH without the service, the daemon lands in
  launchd's background session — no access to the Mac's screen, and browser
  windows open nowhere — so on a daemon machine reached over SSH the person
  should run `equipe daemon install` once; the app can suggest it when
  `started` is `true`. `equipe attach --stdio --no-start` fails instead.
- **Endings** (the exit status of `ssh`):

  | Status | Meaning | The app |
  |---|---|---|
  | 0 | the daemon stopped, or the app hung up | `Down`, retry quietly |
  | 69 | no daemon could be reached or started; why is on stderr | show stderr's line |
  | 127 | no `equipe` on that machine (or not on the `PATH` above) | "install equipe on <host>" |
  | 255 | ssh itself: unknown host key, refused key, no route | show ssh's stderr, "try `ssh <host>` in a terminal" |

  Keep stderr's last line for the banner; nothing else is ever written there by
  the bridge.

### Security

- No new door: the daemon's socket stays a 0600 file in a 0700 folder, and the
  bridge is a process of the user SSH signed in as. Whoever can reach the daemon
  this way could already run any command as that user; nothing is exposed that a
  shell on the machine doesn't already give.
- No new credential: authentication, host keys and encryption are SSH's. The
  app stores only the host name.
- The bridge reads nothing and adds nothing past its first line; the daemon
  checks every request as it does for a local screen.
- `upload` writes only images (`png jpg jpeg gif webp`, ≤ 40 MB) under
  `~/.equipe/uploads`, 0600 in 0700, under a name equipe chooses — the screen
  can't pick where. Uploads older than 7 days are swept when the next one arrives.

### What the app does on this Mac, and over SSH

Everything the app shows comes from `hello` and `event`, so it works remotely
as is. These are the places that touch the disk or start a program on the
machine the app runs on, and what each does when the daemon is elsewhere.
"`ssh` side command" means `ssh <same options> <host> '<PATH prefix> <command>'`
over the same control connection; quote every path with single quotes
(`'…'`, with `'` written `'\''`).

| What | On the daemon's Mac today | Over SSH |
|---|---|---|
| Socket | `daemon::Target::from_env`: `<home>/equipe.sock` | `equipe attach --stdio` (above) |
| Squad list and the squad to open | `state::squad_names` reads `<home>/squads/*/squad.db`; `Target::from_env` takes `$EQUIPE_SQUAD`, else `squads::default_squad` (`state.json`) | **`squads` request** → `{squads: [{name, projects, home}], last}`. Use it locally too, so both paths are one code path. Ask on each attach (it is cheap). |
| Image attach (paste, drop, file) | the file stays where it is; the message carries `[image: /path]` | **`upload` request** `{name, data: base64}` → `{path}`; put `[image: <path>]` in `Submit`. Not `~/.equipe/inbox` (`equipe send-image`'s): a console open on that machine takes those into its own composer. |
| `/home` (open the folder) | `open <home>` (path from `squads` → `home`) | Nothing opens there. Show the path with a copy button. |
| `/home changes`, `/home undo [n]` | `equipe home changes --squad <s>` / `equipe home undo <n> --squad <s>`, run here | the same as an `ssh` side command with `EQUIPE_NO_OPEN=1` in front. Show stdout's lines; exit ≠ 0 → stderr's last non-empty line is the reason. |
| `/memory`, `/memory changes` | `equipe memory [changes] --squad <s>` | the same, `EQUIPE_NO_OPEN=1 equipe memory …`; `EQUIPE_NO_OPEN` keeps it from opening a Finder window on the far screen. |
| A ticket's diff | the ticket (from the snapshot) gives `dir, branch, from`: a `home`-kind ticket (`task`, `action`) → `ticket.worktree`, `branch`, `main`; a `code` ticket → `ticket.project` (else the squad's first project), `branch`, `HEAD`. Stat: `git -C <dir> diff --stat=200 <from>...<branch>`; full: `git -C <dir> diff <from>...<branch>` | the same two `git` commands as `ssh` side commands. Cache per (ticket, branch) as the console does; fetch the full diff only when it is opened. |
| `/export` | the conversation is the app's own (snapshot + events), rendered to Markdown | write it **on the laptop** (the person reads it there): a save panel, defaulting to `~/Downloads/<squad>-<member>-<time>.md`. Nothing goes to the daemon's machine. |
| Browser sign-in (`/browser signin`) | `equipe browser [url]`: a Chrome window on this screen; members then use the saved sign-ins | `equipe browser [url]` as an `ssh` side command: the window opens on **the daemon's machine's** screen, where the members' browser lives. Tell the person to reach it with Screen Sharing (`open vnc://<host>`); the command ends when they close the window. Needs the daemon machine's GUI session (see "Daemon off"). |
| `/browser` (show / watch) | `ShowBrowser` / `WatchBrowser` commands | the same — already protocol. |
| Approval preview (`Approval.preview`) | a file path on the daemon's Mac; open it | `cat '<path>'` as an `ssh` side command into a temp file on the laptop, then show it. It is deleted on the daemon's side once answered, so fetch it when the request arrives. |
| Folder completion, `/squad add`, `/squad new` | lists folders on this disk | not offered remotely yet: point the person to the console on the daemon's machine (`ssh <host>`, `equipe`). |
| Copy to clipboard | the laptop's clipboard | the same — never the daemon's. |

Later, when the desktop needs them often, the `ssh` side commands can become
daemon requests like `squads` and `upload` did (a `diff` request first); the
table is the contract until then.

The old spike (mock agents into SQLite) is kept as `examples/mock_agents.rs`
(`cargo run --release -p equipe-desktop --example mock_agents`); the knobs below
are for it.

Spike knobs (env): `EQUIPE_AGENTS` (3), `EQUIPE_RATE` lines/s per mock agent
(200, `0` = idle), `EQUIPE_TICK_MS` UI poll interval (33), `EQUIPE_W`/`EQUIPE_H`.

## Rules that keep it light

1. **Logs never live in UI memory.** Agents append to SQLite in batched
   transactions (50 ms); each pane is a `uniform_list` that reads only the rows it
   is about to draw, by `(agent, seq)`.
2. **Draw nothing when nothing changed.** On macOS, any process that keeps
   presenting frames holds a ~105 MB GPU working set (see below). It is released
   ~1 s after the last frame. So no clocks, spinners or live readouts that repaint
   on their own; status text rides along with frames that happen anyway.
3. **Coalesce repaints.** The UI polls agent counters at a fixed tick and repaints
   only if something moved. Line rate does not change CPU, the tick does.

## Spike measurements

Apple Silicon, macOS 26.2, release build, 1200×760 window, `phys_footprint`
(what Activity Monitor shows). Single runs on one machine — orders of magnitude,
not benchmarks. 2026-10-11.

| | Result |
|---|---|
| Binary (stripped, thin LTO) | 12 MB (hello world: 9.9 MB, 4.4 MB gzipped) |
| Idle, 3 agent panes, board | **33 MB**, 0.2% CPU |
| Idle, daemon client (this app, sidebar + empty body, no events; 2026-10-14) | **34–38 MB**, 0.1% CPU — no frames between events |
| 3 agents streaming, tick 33 / 66 / 100 ms | 152 MB, 8.7% / 5.5% / 2.7% CPU |
| 3 agents × 3000 lines/s for 90 s (720k lines, 59 MB DB) | 153 MB, flat |
| Startup peak | ~141 MB, transient |
| First build / incremental dev rebuild / incremental release | ~75 s / 0.6 s / 15 s |

### The ~105 MB while drawing is macOS, not GPUI

| Probe (600×400 window) | Never redraws | Redraws every 1 s |
|---|---|---|
| Plain GPUI | 26 MB | 133 MB |
| GPUI Kit | 30 MB | 137 MB |
| Pure Swift + `CAMetalLayer` | 24 MB | 134 MB |
| Swift, Metal → IOSurface as `CALayer.contents` | 24 MB | 134 MB |
| Swift AppKit `draw(_:)`, no Metal | 16 MB | 129 MB |

The extra memory is reported as "Owned physical footprint (unmapped) (graphics)".
It does not scale with window size or window count. GPUI's own Metal allocations
(`device.current_allocated_size()`) stay at ~7 MB. Ghostty shows the same ~108 MB.
Redrawing every 2 s or more lets it drop between frames. So any UI stack that
draws in-process pays this while active. What we control is the idle floor and
how long we stay "active".
