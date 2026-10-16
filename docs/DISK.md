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
"디스크가 가득 차서 frontend의 빌드가 실패했어요 · 정리하기" — once until there's room again;
the member is told not to retry the build.

## What takes the space

Asked for (`disk_scan`, `equipe disk`), within a 20-second budget — never the whole disk:

- **Members' worktrees** (`~/.equipe/worktrees/*/*`), each with its ticket's state, and the
  build output in it apart (`target/`, `node_modules/`, `.next/`, `dist/`, `build/`,
  `DerivedData/`, `.turbo/` up to three levels down, only what git ignores there).
- **Members' build folders** (`~/.equipe/cache/build/<squad>/<member>`, their
  `CARGO_TARGET_DIR`).
- **equipe's own**: logs, the members' browser, journals, uploads.
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
- a log past 20 MB, trimmed to its last 1 MB.

Never: an open ticket's worktree (in progress, waiting, in review, or done but not in main),
a worktree with anything not committed, anything outside `~/.equipe`, anything reached
through a link, the person's projects or caches. A screen names items by the id of the
daemon's own look, never by path.

When a ticket is merged or cancelled, build output left in a worktree of its goes at once,
on its own (the worktree and branch stay), and `daemon.log` says so.

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
- Desktop: the banner's 정리하기, settings → 스쿼드와 기기 → 디스크, `/disk`, a news row's
  정리하기. TUI: a warning in the header; `/disk` says where things stand.
- Phone: `disk_level` events and a push at critical and full (`docs/remote-protocol.md`).
- Screens' RPC: `disk` → `DiskReport`, `disk_clean { ids }` → `DiskCleaned`.
- Fake daemon: `FAKE_DISK=low|critical|full`.
