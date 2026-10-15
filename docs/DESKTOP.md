# Equipe desktop

All-Rust desktop app: `crates/desktop` (UI on [GPUI Kit](https://github.com/longbridge/gpui-kit) 0.7.1,
which pins its GPUI snapshot, `gpui-pre` 0.3.8). It is a window onto the daemon —
a third screen beside the terminal console and the phone — not a second engine.

```bash
cargo run --release -p equipe-desktop
scripts/desktop-dmg.sh          # the app people download: dist/equipe.dmg
```

People get it from **https://github.com/equipe-labs/equipe/releases/latest/download/equipe.dmg**
(see [Shipping the app](#shipping-the-app)).

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

- **Connection.** `daemon::Target` says *where* (`target::Endpoint`: this Mac's
  socket — `$EQUIPE_HOME`, else `~/.equipe`, `equipe.sock` — or an SSH host, see
  [Remote](#remote-a-daemon-over-ssh)) and which squad to prefer (`$EQUIPE_SQUAD`,
  or the one picked in the sidebar). `daemon::spawn` runs a `Link` on a thread of its
  own (small tokio runtime) that connects, follows, and when the daemon goes away
  waits and retries (250 ms doubling to 5 s; 0.5 s to 10 s over SSH) — so a daemon
  turned back on is picked up without a click. It reports only *changes*: a daemon
  that stays down is one `Down`, not one per retry. Both kinds of target run the
  same handshake over a pair of byte streams (`wire.rs`): `squads` → choose the
  squad (the preferred one if it exists, else the daemon's `last`, else the first)
  → `hello`. Nothing on this Mac's disk is read to find squads.
- **Methods used** (the same door the terminal console uses, `equipe_protocol::rpc`):
  `hello {version, squad}` → `Hello {version, snapshot}` once per connection (the
  snapshot replaces whatever the window held), then `event` notifications
  (`Event`: `WorkStarted/WorkEnded`, `ToolUse`, `TextDelta`, `TurnEnded`,
  `ApprovalRequested/Resolved`, `TicketChanged`, `SessionStarted/Closed`, …).
  `command` (notification) is wired (`Link::send`) but no screen sends yet.
- **State.** `state::Model` folds the snapshot and events into the member list and
  the connection (`Connecting` / `Connected` / `Down`). A member "needs you" when it
  has a permission request open or owns a ticket put down for the person. Pure
  logic, unit-tested without a window. The squad list is the daemon's own `squads`
  answer, asked on every attach.
- **Screens.** `src/views/{sidebar,chat,board}.rs`; `chat` and `board` are
  placeholders that later tickets fill in, each in its own file. `main.rs` holds the
  window, the daemon pump and the connected / banner / daemon-off bodies.
- **Drawing.** Updates that land within 100 ms are applied together; a frame is
  requested only if what is on screen differs (`Model::apply` returns whether it
  does). `EQUIPE_LOG_RENDER=1` prints a line per frame with the memory footprint.
  The daemon-off screen says to run `equipe`; the app does not start the daemon.
  When there is nothing to attach to *and* no `equipe` command on this Mac
  (`install::find_cli` — only for the target "this Mac": PATH, then `~/.local/bin`, `/opt/homebrew/bin`,
  `/usr/local/bin`, `~/.cargo/bin` — an app opened from Finder has no shell
  PATH), it shows how to install instead: the install.sh line with a copy
  button, the Homebrew line, and a link to equipe.im.

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

- **Target.** The window shows which machine it is on (title bar, and the sidebar's
  foot, which opens the chooser; menu *연결 대상…* ⌘⇧K). The chooser lists *이 맥*,
  every `Host` in `~/.ssh/config` (and files it `Include`s; no wildcards), the hosts
  typed before, and a box for `host`, `user@host`, `user@host:port` or `[::1]:port`
  — anything `ssh` resolves; `-` at the start, spaces and shell characters are
  refused, and the destination goes after `--`. The last target is remembered in
  `<home>/desktop-target` (`last local` / `last ssh <spec>`, plus `recent ssh …`
  lines) and used at the next launch; `EQUIPE_TARGET` (`local` or a destination)
  overrides it. Nothing else about the machine is assumed: no host, user, path or shell.
- **The command.** The app runs the system `ssh` (the person's own config —
  ProxyJump, IdentityFile, agents, 1Password's agent — applies as is), with no shell
  of the app's in between:

  ```
  ssh -T -o ServerAliveInterval=15 -o ServerAliveCountMax=3 -o ConnectTimeout=15 \
      -o ControlMaster=auto -o ControlPath=/tmp/equipe-desktop-<uid>/c-%C -o ControlPersist=600 \
      -o NumberOfPasswordPrompts=2 [-p <port>] -- [user@]<host> \
      'sh -c '\''PATH="$HOME/.local/bin:$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"; export PATH; exec equipe attach --stdio'\'''
  ```
  (the last argument is one string: `sh -c '…'` with the script in single quotes.)

  - **The remote line** is written to mean the same to the remote *login shell*,
    whatever it is: ssh hands the string to `$SHELL -c`, and bash, zsh, fish and
    (t)csh all read `sh -c '<script>'` alike — the script is POSIX, so `$HOME` and
    `$PATH` are left for `sh` (fish's `$PATH` is a list; `VAR=x cmd` isn't understood
    everywhere). Unit-tested through every shell on the machine running the tests;
    tried by hand against bash, fish and zsh logins on Alpine. The prefix covers
    `install.sh` (`~/.local/bin`), `cargo install` (`~/.cargo/bin`) and Homebrew
    (both prefixes) because a non-login shell lacks them.
  - **`ControlMaster`**: one sign-in serves the attach and the side commands below
    (`ssh … sh -s`, the script on standard input so nothing is quoted for the outer
    shell). It lives 10 minutes past the last use. `/tmp/equipe-desktop-<uid>` is 0700,
    owned by the user, and short (Unix socket paths ≤ ~100 bytes).
  - `$EQUIPE_SSH_OPTIONS` adds options before the host (`-F …`, `-J …`), for odd setups.
- **Authentication, by kind.**
  - *Key, agent, 1Password*: `ssh` does it; nothing is asked.
  - *Password, 2-step codes, key passphrase*: `SSH_ASKPASS` is the app's own binary
    (`SSH_ASKPASS_REQUIRE=force`, macOS 12's OpenSSH 8.6 has it). Started by `ssh`, it
    hands the question over a Unix socket (0600, in the 0700 folder, plus a random
    one-time token in its environment) to the running app, which shows a box over the
    window; the answer goes back to `ssh` on stdout and is dropped — **never stored**.
    The control master means it is asked once, not on every reconnect. Closing the box
    cancels the whole attempt (`ssh` does not re-ask) and the window waits for
    *다시 시도*.
  - *A host this Mac has not met*: never auto-accepted and never an error screen.
    `ssh`'s own question arrives as a box showing the key's fingerprint
    (`SHA256:…`) and the host; *지문이 맞습니다 — 연결* answers yes (and `ssh` writes
    `known_hosts` itself), the other button no. The person's `StrictHostKeyChecking`
    setting is untouched: with `yes` the guide says to `ssh <host>` once in a
    terminal; a *changed* key is refused with a warning (no way around it in the app).
  - With no place for the helper's socket, `BatchMode=yes` keeps `ssh` from asking on
    a terminal; keys and the agent still work.
- **The first line** is a notification the bridge writes before anything the
  daemon says (`rpc::Attached`):

  ```json
  {"jsonrpc":"2.0","method":"attached","params":{"bridge":"0.21.0+1760400000","daemon":"0.21.0+1760400000","protocol":1,"started":false}}
  ```

  Then the app sends `squads`, then `hello {version, squad}`. It compares
  `protocol` with `rpc::PROTOCOL`: equal → carry on; the daemon's lower → "update
  equipe on <host>" (`equipe update`, then `equipe daemon restart`); higher →
  "update this app". An `equipe` without `attach` at all ends `unrecognized
  subcommand`: the same advice. `bridge` ≠ `daemon` means the daemon runs an older
  build than the installed `equipe` — the console's "restart when idle" hint, not an
  error.
- **Daemon off.** The bridge starts it, as the console does: through the login
  service when `equipe daemon install` set one up, else detached (`setsid`),
  and says `"started": true`. Without the service the daemon lands in launchd's
  background session — no access to the Mac's screen, and browser windows open
  nowhere — so when `started` is `true` the window says to run `equipe daemon
  install` once on that machine. (The bridge may take ~20 s to give up on a daemon
  that won't start; the window says "connecting" meanwhile.)
- **Endings and the guide for each.** `ssh`'s exit status and its last lines on stderr
  (`failure::classify_exit`) become a cause; each has its own screen
  (`Failure::guide`) with the steps, the lines to paste (copy buttons) and `ssh`'s
  own message underneath:

  | Seen | Cause | The guide says |
  |---|---|---|
  | `Connection refused` | no SSH server listening | turn on *원격 로그인* (macOS: 시스템 설정 → 일반 → 공유; ≤ 12: 시스템 환경설정 → 공유) / install and enable `sshd` on Linux; custom port as `host:port` |
  | no route, timeout, unknown name | unreachable | check the name, that the machine is awake, VPN / Tailscale, try `ssh` in a terminal |
  | `Permission denied` | authentication failed | `ssh-keygen -t ed25519`, `ssh-copy-id [-p …] user@host`, check the user name |
  | closed password box | sign-in cancelled | try again when ready |
  | `Host key verification failed` | fingerprint not confirmed | try again, compare the fingerprint |
  | …`strict checking` | `StrictHostKeyChecking yes` | `ssh <host>` once in a terminal |
  | `REMOTE HOST IDENTIFICATION HAS CHANGED` | key changed | ask the admin; `ssh-keygen -R <host>` only if it really changed |
  | exit 127 | no `equipe` there (or not on the PATH above) | install line (install.sh / `brew install equipe-labs/tap/equipe`) to run on that machine |
  | exit 69 | daemon unreachable / won't start | stderr's line, `equipe daemon start` |
  | `attached.protocol` ≠ ours, or no `attach` | version mismatch | `equipe update` + `equipe daemon restart` there, or update the app |
  | no squads | nothing to follow | run `equipe` in a project on that machine |
  | exit 0 / a mid-session loss | lost it | retry quietly |

  Causes only the person can fix (cancelled box, auth failure, host-key refusals,
  an app older than the daemon) are **not** retried on a timer — the window would
  keep asking for the same password. They wait for *다시 시도* (the guide's button, or
  the banner's). Everything else — a refused port, an unreachable host, a daemon
  that is off — keeps retrying with backoff and is picked up when it comes back.
- **Reconnecting.** An `ssh` that exits is run again after the backoff; the window
  keeps the last snapshot under a banner meanwhile and replaces it on the next
  `hello`. Cut the network and the control master notices within ~45 s
  (`ServerAlive*`) and the attach ends; when the network is back the next try
  succeeds on its own. A Mac that slept is noticed by the wall clock jumping between
  two 2-second ticks: the link kills its `ssh`, closes the shared connection (it would
  otherwise be reused dead) and reconnects at once.

### Security

- No new door: the daemon's socket stays a 0600 file in a 0700 folder, and the
  bridge is a process of the user SSH signed in as. Whoever can reach the daemon
  this way could already run any command as that user; nothing is exposed that a
  shell on the machine doesn't already give.
- No new credential: authentication, host keys and encryption are SSH's. The
  app stores only which host it last used. A password or code typed in the box goes
  straight to `ssh` and is dropped; the helper socket answers only a caller that
  has the one-time token, and only this user can open it.
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
| Squad list and the squad to open | **done** — the `squads` request → `{squads: [{name, projects, home}], last}`, locally and remotely alike (`wire::connect`), on each attach | the same |
| Image attach (paste, drop, file) | **done** (`attach.rs`, `views/composer.rs`) — ⌘V (clipboard picture or copied image files), drop on the window, and `equipe send-image`'s inbox (a file watch on `<home>/inbox`, this Mac's daemon only) put thumbnails above the box (≤ 5, ≤ 8 MB each, long side shrunk to 1568 px; decoding, shrinking and the upload run on the background pool). Local and SSH alike use the `upload` request (below); `[image: <path>] ` is appended to the message at send | the **`upload` request** `{name, data: base64}` → `{path}`; put `[image: <path>]` in `Submit`. Not `~/.equipe/inbox` (`equipe send-image`'s): a console open on that machine takes those into its own composer. |
| `/home` (open the folder) | `open <home>` (path from `squads` → `home`) | Nothing opens there. Show the path with a copy button. |
| `/home changes`, `/home undo [n]` | `equipe home changes --squad <s>` / `equipe home undo <n> --squad <s>`, run here | the same as an `ssh` side command with `EQUIPE_NO_OPEN=1` in front. Show stdout's lines; exit ≠ 0 → stderr's last non-empty line is the reason. |
| `/memory`, `/memory changes` | `equipe memory [changes] --squad <s>` | the same, `EQUIPE_NO_OPEN=1 equipe memory …`; `EQUIPE_NO_OPEN` keeps it from opening a Finder window on the far screen. |
| A ticket's diff | the ticket (from the snapshot) gives `dir, branch, from`: a `home`-kind ticket (`task`, `action`) → `ticket.worktree`, `branch`, `main`; a `code` ticket → `ticket.project` (else the squad's first project), `branch`, `HEAD`. Stat: `git -C <dir> diff --stat=200 <from>...<branch>`; full: `git -C <dir> diff <from>...<branch>` | the same two `git` commands as `ssh` side commands. Cache per (ticket, branch) as the console does; fetch the full diff only when it is opened. |
| `/export` | the conversation is the app's own (snapshot + events), rendered to Markdown | write it **on the laptop** (the person reads it there): a save panel, defaulting to `~/Downloads/<squad>-<member>-<time>.md`. Nothing goes to the daemon's machine. |
| Browser sign-in (`/browser signin`) | `equipe browser [url]`: a Chrome window on this screen; members then use the saved sign-ins | `equipe browser [url]` as an `ssh` side command: the window opens on **the daemon's machine's** screen, where the members' browser lives. Tell the person to reach it with Screen Sharing (`open vnc://<host>`); the command ends when they close the window. Needs the daemon machine's GUI session (see "Daemon off"). |
| `/browser` (show / watch) | `ShowBrowser` / `WatchBrowser` commands | the same — already protocol. |
| Approval preview (`Approval.preview`) | a file path on the daemon's Mac; open it | **done** (`Shell::open_file`) — `cat -- '<path>'` as a side command into a 0600 temp file in `/tmp/equipe-desktop-<uid>/`, then `open`. Fetched on click while the request is still open (the file goes once it is answered); a failure shows in the flash line. |
| Folder completion, `/squad add`, `/squad new` | lists folders on this disk | not offered remotely yet: point the person to the console on the daemon's machine (`ssh <host>`, `equipe`). |
| Copy to clipboard | the laptop's clipboard | the same — never the daemon's. |

**Trying it.** `cargo run -p equipe-desktop --example remote_probe -- user@host[:port]
[--trust]` runs the same link without a window and prints what it says (a password from
`EQUIPE_PROBE_PASSWORD`, `--trust` to accept a first-seen host key; `local` for this
Mac). With `--features snapshot`, `EQUIPE_TARGET=user@host EQUIPE_SCRIPT="wait:2500;shot:/tmp/a.png;trust;wait:1500;pass:<password>;…"`
drives the window through the host-key and password boxes (`chooser`, `connect:<host>`,
`decline`, `retry` too). A Linux machine to try against: any container with `sshd`
and a stand-in `equipe` that prints the bridge's three lines.

Later, when the desktop needs them often, the `ssh` side commands can become
daemon requests like `squads` and `upload` did (a `diff` request first); the
table is the contract until then.

**The board** (`views/board.rs`; rules in `board.rs`, git in `diffs.rs`). It has the
terminal board's order, lanes, folding, tree (`g`), search (`f`, done tickets too) and
calls (`a` `r` `m` `d` Enter), and changes only on the daemon's events: every action is
the same command the console sends (`accept`, `merge{push}`, `reopen`, `resume`,
`cancel`). The diff and the branch stat are the two `git` commands of the table above —
run here on this Mac, as `ssh` side commands otherwise. The list/detail divider is
dragged and kept in `desktop-target` (`board-split`). To try it on a real daemon without
touching anyone's squads: `EQUIPE_HOME=$(mktemp -d)`, `examples/seed_board` for a squad
with projects, `equipe daemon run` (stopped while tickets are put into
`squads/<name>/squad.db` with `sqlite3`), then `--features snapshot` with
`EQUIPE_SCRIPT="board;pick:2;bkey:d;shot:/tmp/a.png"` (`bkey`, `pick`, `find`, `note`,
`submit`, `cancel` drive the board).

**티켓** (`views/tickets.rs`; rules in `tickets.rs`, the daemon's in
`equipe_core::runtime`'s `tickets.rs` and `equipe_core::board`). Every ticket, the person's to
shape — with their own commands only (`create_ticket`, `edit_ticket`, `edit_tickets`,
`move_ticket`, `comment_ticket`, `ticket_history`); a member's word never moves one.

- *List* (default) or *board* (V; kept as `tickets board` in `desktop-target`). The list runs by
  attention: 내 차례 · 검토 · 진행 중 · 할 일 (the board's order) · 끝남 (the week, folded);
  cancelled shows only when the filter asks. Proposals wait behind "분류 필요 N": a triage of
  their own where A takes one on and N drops it. A row: status (live) · T-n · title · ⛔ what it
  waits on · project · who and what they do now, or their place in their queue (다음 #1) ·
  diff · priority · when it last changed; one waiting for the person is lit, as in 나에게 —
  it is the same list of needs, the same peek, the same keys.
- By hand a ticket goes from proposal to 할 일, moves within 할 일 (what a member picks next is
  that order; priority only between equals), or is cancelled (asked once; on the board, drop it
  on the place at its foot). 진행 중, 내 차례, 검토 and 끝남 are the members' and A's: a drop there
  is refused, saying why. The daemon takes more (`board::move_rule`: review to done accepts,
  done back sends back…), for screens that ask.
- Edits show at once and are sent after the five seconds U can take them back in (a refusal
  puts them back); several chosen go as one `edit_tickets`, so the PM hears once. Work under way
  given to someone else is asked about first: its member stops, its branch stays.
- Keys: C the quick line (`@who #project !priority after T-n title`; ⌘⏎ makes it, ⇧⌘⏎ has
  the PM shape it into a proposal, ⇧C the full sheet), M owner, P priority, ⇧P project,
  S status, B what it waits for, E the title, ⌥V the views, H/L across the board's columns.
- The page (⏎) edits its title and Markdown description in place, its properties at the
  right, and keeps its record (the journal's `ticket_events`) with the person's comments —
  a comment reaches the member on it as the person talking, else its owner and the PM.

The fake daemon does all of these as the daemon does; the snapshot script has `layout:`,
`quick:`, `tsheet` `ttitle:` `tbody:` `tcrit:` `tcreate`, `tmenu:<prio|status|who|project|kind|deps>,<id|sheet|bulk>`,
`page:<id>[,activity]`, `tedit:`, `tbodyedit:`, `tcomment:`, `choose:<ids>` and a drag by hand
(`mdown:` `mmove:` `mup:`).

The old spike (mock agents into SQLite) is kept as `examples/mock_agents.rs`
(`cargo run --release -p equipe-desktop --example mock_agents`); the knobs below
are for it.

Spike knobs (env): `EQUIPE_AGENTS` (3), `EQUIPE_RATE` lines/s per mock agent
(200, `0` = idle), `EQUIPE_TICK_MS` UI poll interval (33), `EQUIPE_W`/`EQUIPE_H`.

## Pairing a phone

"휴대폰 연결…" (the app menu, the account menu at the sidebar's foot, or settings ▸ 스쿼드와
기기 ▸ 휴대폰) opens a sheet with a QR code the equipe app scans — no terminal. The daemon
does the pairing on its own machine, which over SSH is not this Mac: the sheet sends
`pair_start` and draws what comes back (docs/remote-protocol.md, "From a screen"). The stages:
a spinner while the daemon opens a code at the relay; the code (black on white with a quiet
zone, drawn as whole-point rectangles so it scans in light and dark), three steps, its time
left and "링크 복사"; the phone asking, with 거절 / 허용 (the person's yes is still what pairs
it); "연결됨", and the sheet closes itself. A code that runs out is replaced up to three times,
then the sheet offers a new one. Errors say what to do: the relay unreachable, another code
open elsewhere, a daemon without its phone link, or one that never answers (older than the
app: update it, or `equipe pair` meanwhile). The code lives only in the sheet's state while it
is open; closing it cancels the code (or says no to a phone left asking).

Settings lists the paired phones — connected now, or when last seen — with "연결 끊기",
asked twice. The rules are `src/pair.rs`, the drawing `src/views/pair.rs`. The fake daemon
answers pairing too (`FAKE_PAIR=ask|relay|silent`), and the snapshot script has `pair` and
`pair:allow|deny|retry|close`.

## The browser panel

⌘⇧B (or the header's 브라우저) opens the shared Chrome the members browse in,
live, beside the conversation: tabs grouped by member (in its colour) and the
person's own (내 탭), back/forward/reload, the address, Chrome에서 열기, and who
has the page in hand. It follows the conversation on screen. It is not a web
view: the page is the daemon's Chrome (`crates/core/assets/browser-host.cjs`),
signed in once, so the person sees exactly what the members see.

- **Where pages live.** Chrome stops drawing the front tab of a minimized
  window after a cross-site navigation (no screenshot, no screencast); a tab
  *behind* another keeps drawing for DevTools. So members' tabs open as
  background tabs of one minimized "parking" window, behind an equipe
  placeholder — no window comes up, no focus is taken, and screenshots work.
- **Pictures.** A second connection (`browser_stream`, `equipe_protocol::stream`)
  carries JPEG screencast frames of the watched page — only while a panel
  watches it, ~30/s with the window in front, 5/s behind, quality 70 locally,
  50 over SSH (a second `ssh … equipe attach --stdio` on the shared
  connection). Credit-paced: the app gives credit back after drawing, so a
  slow line gets fewer, newer frames. Frames never go on the event connection.
- **The hand.** Mouse, wheel and keys go back through the same stream, mapped
  through the frame's CSS size; Korean composition goes as
  `Input.imeSetComposition`, commits as `insertText`, editing keys carry the
  Mac's editing commands. Touching a member's page takes it (its browser tools
  answer "The person is using this browser…" meanwhile); 돌려주기, Esc or two
  idle minutes give it back. A member's `show_my_browser` opens the panel on
  its browser in the person's hands instead of raising a window when a panel is
  attached; handing back tells the member "The person finished in the browser."
- JS dialogs show as a card over the page; what a picture can't do (passkeys,
  permission prompts, a `<select>`'s list, file choosers) goes through
  Chrome에서 열기.

Snapshot steps: `browser`, `bopen:<url>`, `bpick:<n>`, `bclick:<fx>,<fy>`,
`bkeys:<text>`, `bime:ㅎ|하|한`. The host's ops have a smoke test:
`npx -y -p playwright@1.64.0 node scripts/browser-host-smoke.cjs`.

## Zoom (⌘+ ⌘- ⌘0)

The window zooms as a browser (or Claude) does: 보기 ▸ 확대 (⌘+, also ⌘=),
축소 (⌘-), 실제 크기 (⌘0), through 67 · 75 · 80 · 90 · 100 · 110 · 125 · 150 ·
175 · 200 %. Settings ▸ 일반 ▸ 화면 크기 has the same as − / + and 실제 크기. The
size is kept in `desktop-target` (`zoom 125`) and put back at launch; a small
chip with the new size shows near the window's top for a moment (`zoom.rs`).

Everything scales together because GPUI draws the window larger, not the app:
`Window::set_content_zoom` (an `equipe:` patch in `vendor/gpui-pre`) multiplies
the display's scale factor by the zoom and divides the viewport and every input
position (and pixel scroll deltas) by it, so layout reflows, text and icons are
rasterized at the effective scale (crisp, not stretched), hit-testing agrees
with what is drawn, and the IME's candidate window is placed in the platform's
points. The window's own geometry (`bounds`, `resize`) stays in screen points.
AppKit's traffic lights don't zoom: they stay 14 pt in and move down to the
zoomed title bar's centre line, and the strip's buttons start after them at
every size (`views::sidebar::lights_at`, `lights_end`).

⌘0 is actual size, as everywhere on the Mac, so the project slices are ⌃0–⌃9
(⌃0: every project). Snapshot steps: `zoom:<pct|in|out|actual>`; `click:` and
`hover:` take screen points and go through the zoom as the Mac's events do.

## Shipping the app

Direct download from the website (the Mac App Store build is separate — it
needs the sandbox; see the squad's `mac-app-store-sandbox` note).

### Fixed URLs

| What | URL |
|---|---|
| Download (what the website links to) | `https://github.com/equipe-labs/equipe/releases/latest/download/equipe.dmg` |
| Update feed (in the app's Info.plist) | `https://github.com/equipe-labs/equipe/releases/latest/download/appcast.xml` |
| Checksum | `…/latest/download/equipe.dmg.sha256` |

`latest` is GitHub's newest release that is not a draft or a prerelease, so a
`v1.2.3-rc.1` tag never reaches downloads or updates. Asset names carry no
version, so the links never change. Releases before the desktop app (≤ v0.24.1)
have no `equipe.dmg`: the link 404s until the first release that has one.

### What `scripts/desktop-dmg.sh` makes

One command on a Mac, from the repo root; ~5 min cold (two architectures).

- **`equipe.app`**: `Contents/MacOS/equipe-desktop` is one universal binary
  (`aarch64` + `x86_64` built with `MACOSX_DEPLOYMENT_TARGET=12.0`, joined with
  `lipo`); `Info.plist` with bundle ID `im.equipe.desktop`, version = the
  workspace version (both `CFBundleShortVersionString` and `CFBundleVersion`,
  which is what Sparkle compares), minimum macOS 12.0 (Sparkle 2.10's floor),
  icon `crates/desktop/macos/AppIcon.icns` (T-88, light variant).
- **`equipe.dmg`**: the app and an `Applications` link to drag it onto. Plus
  `equipe.dmg.sha256`.
- **Signing**, inside out (Sparkle's helpers, the framework, the app, the DMG):
  with a `Developer ID Application` identity (`EQUIPE_SIGN_IDENTITY`, else the
  first in the keychain) — Hardened Runtime, secure timestamp, no entitlements.
  Without one — ad hoc, and the script ends with `signed: NO`.
- **Notarization**: when signed and given credentials — `EQUIPE_NOTARY_PROFILE`
  (a `xcrun notarytool store-credentials` profile in the keychain) or
  `EQUIPE_NOTARY_KEY` (.p8 path) + `EQUIPE_NOTARY_KEY_ID` + `EQUIPE_NOTARY_ISSUER`
  — `notarytool submit --wait`, then `stapler staple` on the DMG.
- **Updates** ([Sparkle](https://sparkle-project.org) 2.10.0, fetched once into
  `$CARGO_TARGET_DIR/sparkle/` and checked against a pinned SHA-256): only when the
  update key's public half is known (`SPARKLE_PUBLIC_ED_KEY`, else
  `crates/desktop/macos/sparkle-public-ed-key`). Sparkle.framework goes into
  `Contents/Frameworks` (its XPC services removed — they are for sandboxed apps),
  and Info.plist gets `SUFeedURL`, `SUPublicEDKey`, `SURequireSignedFeed`,
  `SUVerifyUpdateBeforeExtraction`, `SUEnableAutomaticChecks`, `SUAutomaticallyUpdate`
  and an hourly `SUScheduledCheckInterval`: a new version downloads in the background,
  its signature checked, and then the sidebar offers **다시 시작해 업데이트** (Sparkle's
  `updater:willInstallUpdateOnQuit:immediateInstallationBlock:`, kept by our delegate in
  `src/update.rs`): a click installs it and relaunches; quitting installs it anyway.
  The public key is committed (`crates/desktop/macos/sparkle-public-ed-key`). With the private
  half (`SPARKLE_ED_PRIVATE_KEY` = contents of the `generate_keys -x` file, or
  `SPARKLE_ED_KEY_FILE`) it writes `appcast.xml` for this DMG — EdDSA-signed
  enclosure, and the feed itself signed (`sign_update appcast.xml`).
  No public key: no Sparkle, `updates: OFF`, and the menu item is greyed out.

Secrets are read only from the environment or the keychain; nothing secret is in
the repository (the public key file is public by design).

In the app (`src/update.rs`): when `Contents/Frameworks/Sparkle.framework` is
there, the app `dlopen`s it and starts `SPUStandardUpdaterController`. Sparkle
checks the feed once a day by itself and asks before installing; the app menu
(equipe ▸ 업데이트 확인…) checks now. `cargo run` has no framework, so updates
are off and the item is disabled. The menu also has 가리기 (⌘H), 기타 가리기
(⌥⌘H), 모두 보기 and 종료 (⌘Q).

Knobs for testing: `EQUIPE_DMG_ARCHS=aarch64` (one arch, quicker),
`EQUIPE_DMG_OUT`, `EQUIPE_APPCAST_URL` / `EQUIPE_DMG_URL` (point the app and the
appcast at a local server).

### How to release

The desktop app ships with the CLI, same version, same tag:

1. Bump `version` in the workspace `Cargo.toml`, merge to `main`.
2. `git tag vX.Y.Z && git push origin vX.Y.Z`.
3. `.github/workflows/release.yml` runs: the `desktop` job (macos-14) runs
   `scripts/desktop-dmg.sh` with the secrets below, checks the DMG (universal,
   signature valid, `Applications` link) and hands `equipe.dmg`,
   `equipe.dmg.sha256` and `appcast.xml` to `publish`, which puts them on the
   release beside the CLI archives. Installed apps see the new `appcast.xml`
   through the `latest` link within a day, or at once with 업데이트 확인….

Run the workflow by hand (Actions ▸ Release ▸ Run workflow) to build the DMG
as an artifact without publishing.

| Secret (repo › Settings › Secrets › Actions) | What |
|---|---|
| `MACOS_CERTIFICATE` | Developer ID Application certificate + key, `.p12`, base64 |
| `MACOS_CERTIFICATE_PASSWORD` | the `.p12`'s password |
| `APPLE_API_KEY` | App Store Connect API key, the `.p8` file's contents |
| `APPLE_API_KEY_ID`, `APPLE_API_ISSUER` | that key's ID and the issuer ID |
| `SPARKLE_ED_PRIVATE_KEY` | Sparkle's private EdDSA key (`generate_keys -x` output) |

Each is optional: without the certificate the DMG is ad hoc; without the API key
it is signed but not notarized; without the Sparkle key there is no appcast.

### Before signing exists: what people see

Checked 2026-10-14 on macOS 26.2 with an ad hoc universal build, marked as
downloaded (`com.apple.quarantine`, as Safari sets it):
`spctl --assess` rejects both the DMG and the copied app (`no usable
signature`). macOS therefore refuses the first open. Since macOS 15 the
Control-click ▸ Open bypass is gone: the person opens System Settings ▸ Privacy
& Security, clicks **Open Anyway** next to "equipe was blocked", and confirms
with their password (macOS 15+ behaviour as Apple documents it; the dialog was not captured on this
Mac). `xattr -dr com.apple.quarantine /Applications/equipe.app` also works.
Once opened, the app runs normally, and updates work unsigned too (Sparkle
checks its own EdDSA signature, not Apple's) — tested end to end with a
throwaway key against a local feed: 0.24.1 found 0.24.2, downloaded it,
installed it on quit; a feed edited after signing was refused.

### What the person has to do for signed, notarized builds

1. **Decide whose name the app is signed in** (the squad's
   `apple-developer-account` note: the Umbriles Inc. membership lapsed on
   2025-08-12) and renew or enroll in the Apple Developer Program ($99/year).
   The name appears in the "Developer ID Application: …" certificate and in
   macOS's dialogs.
2. **Create a Developer ID Application certificate** — only the Account Holder
   can: Xcode ▸ Settings ▸ Accounts ▸ Manage Certificates ▸ + ▸ Developer ID
   Application (or developer.apple.com ▸ Certificates with a CSR).
3. **Export it** from Keychain Access as `.p12` with a password, then
   `base64 -i cert.p12 | gh secret set MACOS_CERTIFICATE -R equipe-labs/equipe`
   and `gh secret set MACOS_CERTIFICATE_PASSWORD -R equipe-labs/equipe`.
   Delete the `.p12` afterwards; keep it in a password manager.
4. **Create an App Store Connect API key** (App Store Connect ▸ Users and Access
   ▸ Integrations ▸ App Store Connect API ▸ Team Keys, role Developer), download
   the `.p8` (only once), and set `APPLE_API_KEY` (`gh secret set APPLE_API_KEY < AuthKey_XXXX.p8`),
   `APPLE_API_KEY_ID`, `APPLE_API_ISSUER`.
   For signing on this Mac instead: `xcrun notarytool store-credentials equipe-notary --key AuthKey_XXXX.p8 --key-id … --issuer …`
   and run the script with `EQUIPE_NOTARY_PROFILE=equipe-notary`.
5. **Make the update key, once, and never lose it** (independent of Apple — can
   be done today): from Sparkle's release (`gh release download 2.10.0 -R
   sparkle-project/Sparkle -p 'Sparkle-2.10.0.tar.xz'`), `bin/generate_keys`
   prints the public key (commit it as `crates/desktop/macos/sparkle-public-ed-key`)
   and keeps the private key in the login keychain; `bin/generate_keys -x key.txt`
   exports it for `gh secret set SPARKLE_ED_PRIVATE_KEY < key.txt` (then delete
   `key.txt`, back it up in a password manager). Losing it means installed apps
   can never be updated again.

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
