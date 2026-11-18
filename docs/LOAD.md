# A busy machine

equipe often shares its Mac with other heavy work: a CI runner's jobs, a dozen headless browsers, a
dozen member engines. On 2026-11-18 a Mac mini with 15 cores ran at a load average of 48–63 with about
100 MB of memory free. The daemon still worked, but slowly: a minute tick took 20 s, a screen's
`hello` took 10 s, and the owner's app on another Mac gave up after 5 s and reported the daemon down.
A member's whole-disk search (`find / -path '*design/sessions/*' …`, run by Claude Code as `bfs`) was
one of the processes using the most CPU.

This page covers what equipe does about that. Code: `crates/core/src/journal.rs` (`Journal::board`),
`crates/core/src/machine_load.rs` and `crates/core/src/runtime/machine.rs` (pacing),
`crates/core/src/guard.rs` (whole-disk searches), `crates/desktop/src/wire.rs` and
`crates/cli/src/remote/link.rs` (waiting for `hello`).

## The board stays in memory

A squad's board grows big: 3,700 tickets carried 45 MB of text, 43 MB of it the notes of closed
tickets. The daemon's loop reads the board many times a minute (dispatch, the minute tick's idle
boards, every screen's snapshot). It used to read the whole board again after any change, and with a
dozen runs heartbeating, the board changes all the time.

`Journal::board` now reads the board once and then updates it one ticket at a time:

- A change made through the daemon's own connection is tracked by temporary triggers
  (`temp.board_touched`), and only the tickets it touched are read again.
- A change made through another connection (another process) re-reads the open tickets, any new
  ticket, and the tickets that were open last time. A closed ticket changed elsewhere is picked up by
  the next whole read, at most every 10 minutes (`BOARD_FULL_READ_EVERY`).
- The board is shared (`Arc<Vec<Ticket>>`), so asking for it doesn't copy it. A snapshot copies only
  the tickets it sends (open ones plus the latest closed ones). Dispatch copies closed tickets without
  their notes and bodies (`board::light_copy`), because it only reads their status and links.

## Screens wait for a slow `hello`

A `hello` answers with the squad's snapshot, and on a busy Mac that takes seconds. A slow answer is
not a dead daemon:

- The app waits up to 90 s for `hello` (`HELLO_TIMEOUT`). Every other handshake step still has 5 s.
- The relay follows each squad for pushes in a task of its own, so a slow daemon no longer blocks
  phones' traffic. It waits up to 90 s and tries again on a later tick if the daemon didn't answer.

## New work waits while the machine is overloaded

Once a minute the daemon reads the machine's load averages (`getloadavg`: the last minute's and the last five
minutes') and, on macOS, the memory level (`kern.memorystatus_level`). On Linux it reads `MemAvailable`. Neither
starts a process, since starting a process on an overloaded Mac can itself take seconds.

- **Overloaded:** memory at 5% or less, or the 1-minute load above 2.5× the cores *and still rising* (at or above
  the 5-minute average), on two readings in a row.
- **Calm:** more than 10% of memory left, and the load falling (below the 5-minute average) or under 2× the
  cores, on two readings in a row.

The first version held work back past 1.5× the cores. On a Mac shared with a CI runner, the load sits at 4–5× the
cores, so new work never started there. Now only a rising load holds work, and work goes again as soon as the load
falls.

While the machine is overloaded:

- **What the person asked for always starts.** That covers their own tickets, the tickets serving their open
  requests, their messages, Carry on, and design-session rounds. Pacing never touches a person's message.
- **Each member with nothing running still gets one ticket.** A member is never left idle on a Mac that is always
  busy. The rest of the board waits.
- Equipe's own background work waits: routines, idle wakes and reminders handed to the PM, signals, daily design
  reviews, and repacks of squad folders (unless the disk is short). A queued design precheck waits to launch its
  browsers, for at most 2 hours.

Turns already running are never stopped. The person sees one notice when the machine becomes overloaded and
another when it calms down.

### What a held turn says

A run's turn starts only once its workspace is made. On a busy Mac that can take minutes, because a project's
`git worktree add`s run one at a time. The screen used to show "Thinking 0s" all that time. Now the daemon tells
each waiting run what it waits on (`Event::SessionWaiting`), and the status row shows it instead, for example
"Making its workspace — 3 ahead of it in site — mini is overloaded (load 77 on 15 cores)". The daemon sends this
again every minute so that screens attached later see it too.

- A run whose worktree already exists (sent back, carried on) or that needs none no longer queues behind other
  tickets' `git worktree add`: it is ready at once.
- When the person writes to a run that is still waiting in line, its workspace goes to the front.

## What members leave behind

A member's command can leave a process running after the shell that started it is gone. The system takes the
process over (its parent becomes 1). Dev servers, mock servers and Playwright scripts collected this way: one ran
for 19 hours at 64% CPU.

Every member command now carries `EQUIPE_RUN=<squad>:<session>:<ticket>`, and child processes inherit it. Every 5
minutes the daemon looks at this user's processes that the system took over and that carry `EQUIPE_MEMBER` (so
equipe started them), and decides by their ticket (`leftovers::orphans`, `leftovers::verdict`):

- The ticket is closed: the process is stopped.
- The ticket is out of work but not closed or in review (put back to do, waiting on the person): the process is
  stopped. It is a dev server or watcher left over from a run that ended.
- The ticket is in progress or in review: the process is left alone, since QA or the person may be looking at what it
  serves. A process whose ticket can't be told is left alone too.

When a process's environment can't be read (recent macOS rarely shows another process's environment), the ticket is
told by the folder it stands in: a ticket's worktree, or its temporary folder.

There's also a cap on dev servers (`next-server`, `vite`, `webpack serve`, `astro dev`, `storybook`, `tsc --watch`,
`nodemon`, …). At most 3 left-over ones stay per project; past that, the oldest go (`leftovers::over_the_cap`). The
daemon doesn't stop a server just because its lane is idle between turns: the next turn usually needs it.

## Spotlight stays out of equipe's copies

Spotlight indexed worktrees and their `node_modules` (`mdworker_shared` ×14 plus `mds_stores`, about 0.7 of a core).
The daemon now drops `.metadata_never_index` in `~/.equipe/worktrees`, `tmp`, `cache` and `design-kit`. That is an
app-level marker in equipe's own folders; no system setting changes. The squads' own folders stay findable.

## Checks of what changed while working

Engineers are told to keep their checks light while they work: check only what changed (`tsc --incremental` with
its `.tsbuildinfo` kept, `vitest related <files>`, `jest --findRelatedTests <files>`, `cargo test -p <crate>`), and
run the full suite once, right before handing in (`roles::ENGINEER_LIGHT`). They are also told to stop the dev
servers and watchers they started, and that one per project is enough.

## Dependencies are cloned before the first turn

A new worktree gets the project's `node_modules` (and other dependency folders) as copy-on-write clones from a
checkout with the same lockfile (`worktree::seed_dependencies`). This used to happen in the background after the
turn had started, so members ran `npm ci` first. Now it happens before the run's first turn. Even then, members ran
`npm ci` over a clone that was already installed. `npm ci` deletes `node_modules` and downloads everything again,
so the guard now refuses it when npm's own record of what is installed (`node_modules/.package-lock.json`) matches
`package-lock.json`. It still allows `npm install <package>`, or a reinstall right after `rm -rf node_modules`.

The practice, for a project's dependencies on a squad's Mac:

1. A new worktree gets `node_modules` as an APFS clone (`clonefile`) of a checkout's, but only when the lockfile is
   byte-for-byte the same. A clone costs no download and almost no disk.
   - The source is checked before cloning. An empty `node_modules` (an install cut short), or one whose
     `.package-lock.json` doesn't match `package-lock.json`, is installed again in that checkout first, then cloned.
     That repair runs `nice npm ci`, one at a time across the daemon, and is not retried for an hour after a
     failure. The log gets one line: "repaired …'s node_modules once (it was empty): npm ci took 7s; worktrees clone
     it from now on".
   - On production, one empty source `node_modules` was cloned into 182 worktrees, and a project script then ran
     `npm ci` in each of them.
   - A link inside it that points into the checkout by an absolute path is pointed at the same place in the worktree.
     The whole folder used to be left out instead.
2. `npm ci` only when the lockfile changed: the ticket's branch adds or bumps a package.
3. To add a package, use `npm install <package>` on top of the clone.
4. A project that uses pnpm shares one store across all worktrees (pnpm's default content-addressed store), so even
   a changed lockfile installs by hard links rather than downloads.

## Members don't search the whole disk

The guard (`equipe hook`, Claude Code's `PreToolUse`) refuses searches that start at the whole disk
or the home folder. This covers `find`/`bfs`/`fd`/`rg`/`ag`/`grep -r`/`du`/`ls -R`/`tree` rooted at
`/`, `/System/Volumes/Data`, `/Users`, `/home`, `/Volumes`, `/private`, `~` or `$HOME`, including
`/*`. A shallow look is allowed (`find / -maxdepth 1`, `tree -L 2 /`). The refusal tells the member
where to search instead: the folder it works in, and for a design session, `design/sessions/` there,
with `ls -t` to list the latest files first.

Design sessions also stop the search from being needed. Each round's note tells the designer where
its files are (`design::session::WHERE_FILES`), and so does the kickoff.
