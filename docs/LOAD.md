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

Once a minute the daemon reads the machine's load average (`getloadavg`) and, on macOS, the memory
level (`kern.memorystatus_level`). On Linux it reads `MemAvailable`. Neither starts a process, since
starting a process on an overloaded Mac can itself take seconds.

- **Overloaded:** the 1-minute load is above 1.5× the cores, or 5% of memory or less is left, on two
  readings in a row.
- **Calm:** the load is below 1.1× the cores and more than 10% of memory is left, on two readings in a
  row.

While the machine is overloaded, equipe starts no new work of its own: no ticket runs or lanes, no
routines, no idle wakes, reminders or news handed to the PM, no signals, no daily design reviews, and
no repacks of squad folders (unless the disk is short). A design precheck that is already queued
waits to launch its browsers, and its QA ticket stays held. It goes ahead anyway after 2 hours.

Turns already running are never stopped, and messages the person sends still go through. The person
sees one notice when the machine becomes overloaded ("This machine is overloaded (load 60 on 15
cores)…", or the low-memory version) and another when it calms down. When it calms down, equipe
dispatches again and the held work starts on its own.

Only a daemon that watches the real machine does this (`Config::watch_disk`). Test runtimes don't.

## Members don't search the whole disk

The guard (`equipe hook`, Claude Code's `PreToolUse`) refuses searches that start at the whole disk
or the home folder. This covers `find`/`bfs`/`fd`/`rg`/`ag`/`grep -r`/`du`/`ls -R`/`tree` rooted at
`/`, `/System/Volumes/Data`, `/Users`, `/home`, `/Volumes`, `/private`, `~` or `$HOME`, including
`/*`. A shallow look is allowed (`find / -maxdepth 1`, `tree -L 2 /`). The refusal tells the member
where to search instead: the folder it works in, and for a design session, `design/sessions/` there,
with `ls -t` to list the latest files first.

Design sessions also stop the search from being needed. Each round's note tells the designer where
its files are (`design::session::WHERE_FILES`), and so does the kickoff.
