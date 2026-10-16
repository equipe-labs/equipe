# Incident 2026-10-16: the daemon stops answering

**Impact.** For about half an hour the desktop app, connected over SSH, showed
"Lost the connection … Reconnecting". `equipe daemon status` on the machine
hung for minutes on `~/.equipe/equipe.sock`. The agents kept working: two
`claude -p` member turns and the browser host were still running. But no
screen could attach, and no member's board tool call got an answer.

## Timeline (KST)

| When | What |
|---|---|
| before 13:00 | Nine coding agents compile at once (load average up to 316 on 12 cores). The disk reaches 100% (ENOSPC) for a while. The daemon clears members' build caches (`daemon.log`: `cleared …/cache/build/…`). |
| ~13:0x | `daemon.log` repeats `remote: the relay said "frame is too large"`. |
| 13:2x | The daemon stops answering. The process is alive at 0% CPU and its workers are parked. Short-lived `git -C <repo> merge-tree --write-tree --no-messages refs/heads/main …` processes keep appearing under it. The desktop retries `equipe attach --stdio` every ~10 s, and those bridges pile up (38 of them). |
| 13:31 | First restart (`kill -TERM`; launchd starts 0.49.0 again). It answers… |
| ~13:33 | …and hangs again within two minutes, at its first minute ticks. |
| 13:53 | Restart on a local hotfix (`db4c1ff`: the per-minute sweep is taken off the core loop). It stays responsive. |
| after | This fix: the sweep returns, off the loop and cached, along with the safeguards below. |

## Root cause

Everything the daemon does for a screen goes through **one** async task, the
runtime's core loop (`crates/core/src/runtime.rs`, `run`). A hello needs a
snapshot from it, `status` needs a `busy` answer from it, and a member's
`mcp__equipe__*` tool call needs it too. Whatever a step of that loop
*blocks* on, every screen waits on.

The minute tick (0.41–0.43, "is the branch already in main") called
`settle_all_merged()` **on that loop**. It ran `merged_state` → `worktree::is_merged`
for every done ticket off main in every squad. Each check ran about seven
synchronous git processes (`rev-parse --show-toplevel`, `symbolic-ref`,
`rev-parse --verify` ×1–2, `merge-base --is-ancestor`, `rev-parse ^{tree}`,
`merge-tree --write-tree`) with `std::process::Command::output()`, one after
another. Nothing remembered an earlier answer, so the same checks ran again
every minute even when no branch had moved. Merging and accepting checked
again on the loop as well.

* **Measured:** the 0.49 journals (read-only copies) hold 29 + 4 done tickets
  with a branch across `equipe-squad` and `cj-geo-squad`. Of those, 11 + 1
  have `merged = 0`, which is what the sweep looks at. One `merge-tree` on
  this repository takes 80 ms on an idle machine, and each check spawns about
  seven processes.
* **Under load average 300 and a full disk**, each process start and each
  merge took far longer. The sweep grew to fill most of every minute. Tokio's
  intervals then caught up on missed ticks in a **burst**
  (`MissedTickBehavior::Burst` is the default), so a stalled loop ran several
  sweeps back to back once it moved again. A hello or a `busy` request was
  served only between sweeps, and the screens timed out before that.
* **Why it hung again at 13:33:** the first minute tick fires immediately on
  start. A fresh 0.49.0 daemon was back in the same sweep within two minutes.
  Only taking the call off the loop (13:53) kept it up.
* **Why the bridges piled up:** `equipe attach --stdio` copies bytes both ways
  and exits only when the daemon closes its side. The daemon's connection
  task was waiting on the stuck loop (`core.subscribe(...).await`) and never
  read the screen's EOF. So each retry of the desktop left one more bridge
  behind.
* **Why `status` hung for minutes:** `equipe daemon status` connected (the
  accept loop was fine), then waited on `busy`, which waited on the core loop
  with no timeout.

"0% CPU, workers parked" fits this: the loop's worker spends its time waiting
on git children. The rest of the runtime has nothing to do, because every
request is queued behind the loop.

### Contributing problems found on the way

These did not cause the incident, but each one could wedge or kill the daemon
in a similar situation:

1. **Untimed awaits on the core loop:** `claude auth status` (on every member
   start), the Codex token refresh and model list (reqwest with no timeout),
   and `git push` on a merge with push. That last one is a credential prompt
   that nobody sees when the person is on SSH.
2. **The browser host's stdin** was written synchronously under the host's
   `std::sync::Mutex`, and sometimes under the hub's lock too, from the core
   loop (`take_frames`, `close_idle_browsers`) and from browser-panel tasks. A
   host that stops reading fills the pipe, and the write then blocks forever
   with the locks held.
3. **`eprintln!` panics when stderr can't be written.** With `daemon.log` on a
   full disk, any log line (`cleared …`, `remote: …`, `telegram: …`) would
   panic. Release builds use `panic = "abort"`, so that stops the whole daemon.
4. **The relay loop:** a squad's hello snapshot (200 recent lines per
   conversation) can be larger than the relay's 1 MiB frame limit. The relay
   drops the frame and answers `frame is too large`. The phone never gets its
   hello, so it asks again, and every retry builds another snapshot on the
   core loop.
5. **Slow screens:** each screen's outgoing queue was unbounded, and a write
   had no deadline. A screen that stopped reading (a stalled SSH link) kept
   every event in memory, browser pictures included.
6. **ENOSPC:** journal writes failed with "database or disk is full", and the
   errors were ignored. No locks were left behind (checked by test), but
   nobody was told.

## Fix

**Off the loop**

* `settle_all_merged` builds a list (squad, ticket, project, branch) on the
  loop and hands it to `spawn_blocking`. The result comes back as
  `Internal::Settled`. Only one sweep runs at a time; if another is asked for
  while one is under way, it runs right after.
  * `worktree::MergedCache` remembers each answer by (repository, branch tip,
    main tip). A sweep then costs `rev-parse --show-toplevel`, `main_branch` and
    one `for-each-ref` per repository, and works a merge out only for a branch
    (or a main) that moved.
  * The periodic sweep runs every **10 minutes**, not every minute. A merge or
    an accept triggers one at once.
  * At accept time, `merged_state` no longer calls git. It answers "not yet"
    when the branch is in a repository, and the sweep settles it.
* The person's **merge** (`merge_into_main` + `push_main`) runs in
  `spawn_blocking`. Its result comes back as `Internal::Merged`, and a merge
  already under way for the same ticket is not started twice.
* **A debug assert:** `worktree::is_merged`, `MergedCache::merged` and
  `push_main` stop a debug build when they are called on the core loop. The
  loop's task carries a tokio task-local (`runtime::on_core_loop`), and a test
  checks both cases.
* **Every git command has a deadline:** 60 s locally and 180 s for a push,
  through `worktree::output_within`. On timeout it kills the command's process
  group. Git also runs with `GIT_TERMINAL_PROMPT=0`. The squad home's git uses
  the same helper.
* `claude auth status` gets a 20 s timeout and is killed on timeout. The
  Codex refresh and model list in the loop get 30 s, and the reqwest client a
  15 s connect timeout.
* The browser host's requests go through a bounded queue to a writer thread
  of their own. `Host::send` never blocks. When the host stops reading, extra
  requests are dropped, and the log says so once.
* The loop's intervals use `MissedTickBehavior::Delay`, so they never
  catch up in a burst.

**Screens**

* Each screen's writer has a 30 s stall deadline per line and a 20 000-line
  backlog cap. Past either, the screen is let go, and it reconnects with a
  fresh snapshot. The reader ends with it.
* A hello waits at most 30 s for the snapshot. Then it gets an error that
  names what the core is stuck on and since when, with `equipe daemon restart`.
* A failed `accept()` (EMFILE, for example) is logged and the loop keeps
  going, instead of stopping the daemon.
* `equipe attach --stdio` gives the daemon 5 s after the screen hangs up, then
  exits. It also gives up on a daemon that doesn't answer `version` within
  10 s. This stops the pile-up of bridges.
* The phone link and the Telegram bridge wait at most 20 s to attach.

**Watchdog and status**

* `watchdog::Pulse` records each loop step: `command merge`,
  `tool create_ticket for acme/pm`, `minute tick: helpers`, and so on.
* A thread of its own (outside tokio, so it runs even when every worker is
  stuck) writes a step that runs longer than 15 s to `daemon.log`, then again
  every minute while it lasts:
  `equipe: watchdog: the daemon hasn't answered for 42s (since 13:21:05): it is on minute tick: … — screens wait meanwhile; `equipe daemon restart` if it stays`.
  When the step ends, it logs that too.
* `busy` answers within 3 s whatever the loop is doing. When the loop doesn't
  answer, the reply carries `stalled { doing, since, secs }` (a new optional
  field in `rpc::Busy`).
* `equipe daemon status` never waits longer than 5 s.
  * A stalled core shows as `응답 없음  13:21:05부터 — minute tick 처리 중 (95초째)`
    with the restart hint.
  * A daemon that answers nothing at all says so and suggests
    `equipe daemon restart`.
* `equipe daemon stop` and `restart` wait 15 s for a graceful stop. After
  that they end the process: the pid comes from the socket's peer credentials,
  and it gets TERM, then KILL. A stop that went through the core also gives
  up on the core after 10 s.

**Disk full and the relay**

* The daemon's own log lines go through `daemon_log!`, which ignores write
  errors (unlike `eprintln!`).
* A journal write failing with SQLITE_FULL or ENOSPC marks the disk as full.
  So does a once-a-minute `statvfs` that finds less than 256 MB free. The
  daemon then logs it once and tells every screen once, including screens
  that attach later (`error.disk_full`). It keeps serving. Once there is more
  than 1 GB free again, it says so (`notice.disk_room`).
* Before sending to a phone, the remote link checks each frame's size. A
  hello snapshot too large for the relay keeps fewer of each conversation's
  recent lines (50, then 10, then none), so the phone gets its answer and
  stops asking. A single event too big for any frame is dropped, and the log
  says so once. The relay's error frames are logged at the 1st, 10th, 100th…
  occurrence instead of every time.

## Tests

* `crates/core/tests/hang.rs` runs git through a stand-in that logs each call
  and makes `merge-tree` take 6 s:
  * While a merge waits on git, `busy` answers within 2 s and a hello within 2 s.
  * After an accept, the sweep doesn't work T-1's merge out again, because
    neither its branch nor main moved.
  * A `claude auth status` that takes 8 s holds up the loop. `busy` still
    answers within its wait and says `command start` is stuck.
* `crates/core/tests/disk_full.rs` uses journals that can't grow (SQLite
  `max_page_count`, which answers SQLITE_FULL). The person is told exactly
  once, turns still run, and `busy` still answers.
* Unit tests:
  * a git command that never ends is stopped at its deadline, children too;
  * `MergedCache` answers again when main or a branch moves;
  * a browser host that never reads doesn't block the sender;
  * a screen that never reads, or falls 10 000 lines behind, is let go;
  * a journal with no room says so and still reads;
  * a too-large hello is trimmed to fit the relay, and a huge event is dropped;
  * the watchdog's pulse reports only a step that is under way;
  * the debug assert fires on the core loop.

## Still on the loop

Shorter git work still runs synchronously on the loop, now bounded by the 60 s
git deadline: worktree create and resume when work starts, `commit_all` when
work is handed in, the squad home's saves, and landing a task in the squad
home. Each of these is a few local git commands, run once per action, not on
a timer. The watchdog names any of them that stalls. Moving them off the loop
is the next step if one ever shows up in `daemon.log`.
