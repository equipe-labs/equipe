# The disk

Members build, and builds are big: a Rust `target/` is 4–15 GB. On 2026-10-16 the disk
filled to 100% several times. Since then equipe watches the room it has, says so before
builds fail, shows what takes the space, and gives back what is its own on the person's
click. Code: `crates/core/src/disk.rs` (levels, the look, the clean),
`crates/core/src/runtime/room.rs` (the daemon's side), `crates/protocol/src/disk.rs` (the
shapes), `crates/desktop/src/disk.rs` + `views/disk.rs` (the banner and the panel),
`crates/cli/src/disk.rs` (`equipe disk`).

## Watching

Every minute, off the core loop (`spawn_blocking`), one `statvfs` for each volume equipe
writes to: `~/.equipe` (its worktrees and build folders are under it) and every squad's
projects. The tightest one counts.

| level | when | what happens |
|---|---|---|
| `ok` | room enough | nothing |
| `low` | under 15 GB free, or a tenth of the disk if that is less | the screens show a banner |
| `critical` | under 5 GB | the banner; every member is briefed, the PM above all (news row in its conversation); **new code tickets wait** (tasks don't build and still start); a push to the phone |
| `full` | under 1 GB, or a write failed for want of room in the last 5 minutes (a journal's SQLITE_FULL, a member's `No space left on device`) | as critical, and the old `error.disk_full` line |

A level is left only with a fifth more room than it took to enter it (critical until 6 GB,
low until 18 GB), so a disk at an edge changes level once. Screens hear only changes
(`disk_level`), and the snapshot carries the level while it is short. When the disk has room
again, members are told and the held code tickets start.

A member's tool or turn that says `No space left on device` (or `ENOSPC`, `os error 28`)
makes the level full at once and puts a plain news row in that member's conversation —
"The disk is full, so frontend's build failed · Clean up" — once until there's room again;
the member is told not to retry the build.

## What takes the space

Asked for (`disk_scan`, `equipe disk`), within a 20-second budget — never the whole disk:

- **Members' worktrees** (`~/.equipe/worktrees/*/*`), each with its ticket's state, and the
  build output in it apart (`target/`, `node_modules/`, `.next/`, `dist/`, `build/`,
  `DerivedData/`, `.turbo/`, SwiftPM's `.build/`, Gradle's project `.gradle/` up to three
  levels down, only what git ignores there).
- **Members' build folders** (`~/.equipe/cache/build/<squad>/<member>`, their
  `CARGO_TARGET_DIR`).
- **equipe's own**: logs, the members' browser, journals, uploads — the attachments of sent
  messages apart (kept until the person removes them; those sent over 30 days ago are offered
  as a fix of their own, never part of "clean everything", never given back on equipe's own),
  and each ticket's phone
  recordings (`<squad home>/recordings/T-<n>/`, given back once the ticket is done or
  cancelled; see [mobile-qa.md](./mobile-qa.md)).
- **Finished tickets' temporary folders** in the system's temporary folders (`/tmp`, which is
  `/private/tmp` on a Mac, and `$TMPDIR`): scratch copies of a branch a reviewer built, a
  ticket's own Gradle home, screenshots. Only entries whose name ties them to a ticket that
  is done or cancelled in **every** squad that has that number (see *Temporary folders*
  below).
- **Build output in the squads' projects**, other tools' worktrees there included
  (`.claude/worktrees/*`, `.codex/worktrees/*`): the person's, so reported with a command.
- **The Mac's caches**, reported only, with the command that clears each: Xcode DerivedData,
  simulators, Cargo's registry, npm, pnpm, Yarn, Docker, the biggest of `~/Library/Caches`.
- The APFS container's free space (`diskutil info -plist`).

Links are never followed; a size that ran out of time says "at least".

## Giving it back

Only on the person's click, after a sheet that lists exactly what goes and the total
(`disk_clean { ids }`, or `equipe disk clean`, which asks y/N). The daemon looks again first
and acts only on what that fresh look still says is safe:

- build output in the worktree of a ticket that is done **and in main**, or cancelled, or
  that no ticket keeps;
- such a worktree itself (`git worktree remove`; **its branch stays**);
- a member's build folder while that member isn't mid-turn;
- a log past 20 MB, trimmed to its last 1 MB;
- attachments of messages sent over 30 days ago, only when the person picks that fix — the
  words of their text files stay in the conversation;
- a finished ticket's `equipe-t<N>-…` temporary folder (below).

Never: an open ticket's worktree (in progress, waiting, in review, or done but not in main),
a worktree with anything not committed, anything outside `~/.equipe`, anything reached
through a link, the person's projects or caches. A screen names items by the id of the
daemon's own look, never by path.

When a ticket is merged or cancelled, build output left in a worktree of its goes at once,
on its own (the worktree and branch stay), and `daemon.log` says so.

## Temporary folders

Members and reviewers make scratch folders outside their worktree: a design QA unpacks and
builds a branch's copy, a mobile ticket gives Gradle a home of its own
(`GRADLE_USER_HOME=/tmp/equipe-t<N>-gradle`, about 2 GB, with a Gradle daemon that outlives
the ticket). On 2026-11-10 these, not the build caches, took the disk down to 2.2 GB.

**The naming rule:** a folder made for ticket T-N outside its worktree goes right in the
system's temporary folder and is named `equipe-t<N>-<what>` (`equipe-t430-gradle`,
`equipe-t506-shots`). That name is what lets equipe give it back on its own:

- `equipe-t<N>…` (alone, or followed by `-`, `_` or `.`), right in `/tmp` or `$TMPDIR`, not a
  link, whose ticket number is done or cancelled in every squad that has it, and untouched
  for an hour: **certain** — it goes with the automatic clean (every few hours, and at once
  when the disk runs short), and is listed in `equipe disk` and `/disk`. A Gradle daemon
  still running from it (one that wrote `daemon/<version>/daemon-<pid>.out.log` there, and
  whose command line is a `GradleDaemon`) is stopped first, politely and then for good.
- touched within the hour: listed as in use, left alone.
- `t<N>-…` (the dash required: never `t16`, `t17venv` or `t1.png`): members often name a
  copy so, but so might the person. It is only **reported**, with the `rm -rf` command that
  removes it, for the person to run from `/disk`; equipe never removes it itself.
- A ticket number that any squad still has open, or that no squad has, is never touched.

A daemon given a person's home of its own (a test's, a demo's) never looks at the system's
temporary folders. The checks equipe runs before the PM's acceptance lands
([USAGE.md](USAGE.md#project-checks)) work in `~/.equipe/checks/`, removed after each run;
what a run cut short left there goes at the next start.

## Leaner builds (the defaults)

- Each member builds in one folder of its own, whatever the ticket
  (`CARGO_TARGET_DIR=~/.equipe/cache/build/<squad>/<member>`), not one `target/` per worktree;
  a member's folder past 10 GB loses its incremental state, then all of it, when the member
  isn't building. Shared per squad it would be smaller still, but two members building at
  once would overwrite each other's binaries.
- `CARGO_PROFILE_DEV_DEBUG=line-tables-only` for members' builds (the test profile follows):
  backtraces still name files and lines; debug information is most of a debug target. Per
  squad: `equipe disk debug lines|off|full` (`off` = none at all, `full` = the project's own
  profile), from the member's next turn.
- Other ecosystems: nothing equipe can set safely for all of them (`node_modules` is the
  project's); their build output in finished worktrees is cleaned as above.

## Commands

- `equipe disk` — the status, what takes the space by group, and what to do.
- `equipe disk clean [--yes]` — what would go, and a y/N.
- `equipe disk debug [lines|off|full] [--squad <name>]`.
- Desktop: the banner's Clean up, settings → Squad & machine → Disk, `/disk`, a news row's
  Clean up. TUI: a warning in the header; `/disk` says where things stand.
- Phone: `disk_level` events and a push at critical and full (`docs/remote-protocol.md`).
- Screens' RPC: `disk` → `DiskReport`, `disk_clean { ids }` → `DiskCleaned`.
- Fake daemon: `FAKE_DISK=low|critical|full`.
