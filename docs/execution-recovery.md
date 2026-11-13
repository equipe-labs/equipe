# Ticket execution recovery

Ticket workflow (`todo`, `in_progress`, `waiting`, `review`, …) describes the
work product. It is deliberately separate from the runtime execution state
(`active`, `queued`, `waiting`, `stalled`, `stopped`). A ticket may therefore
remain `in_progress` while its last run is `stalled`; recovery never creates a
replacement ticket.

## Root cause

Previously, `in_progress` and an open member session were treated as proof
that an engine was running. A turn could end without a terminal board call
(no hand-in, no wait), leaving the ticket assigned to a member that would
never touch it again and that took no other work.

## Runs and their states

Each start owns a durable `(ticket, generation, session, operation)` record:

| State     | Meaning                                                              |
|-----------|----------------------------------------------------------------------|
| `active`  | A runtime session holds the lease.                                   |
| `queued`  | The person approved a recovery; its owner starts it when free.       |
| `waiting` | The run put the ticket down for the person (`block_on_person`).      |
| `stalled` | The run ended without a hand-in or a wait; only the person recovers. |
| `stopped` | The run ended as the workflow said: handed in, accepted, cancelled, reassigned, handed over to another engine. |

Every start gets a fresh runtime session id and the next generation, even on
the same member and engine (Codex to Codex included). Heartbeats, finishes and
the stall check only touch the exact generation (and session) they name, so a
late message from an older run can't revive, extend or close a newer one.

`reason` is a code: `lease_expired`, `not_resumed` or `migrated` for a stalled
run (`equipe_protocol::stall_reason`), and `waiting_for_person`, `handed_in`,
`accepted`, `cancelled`, `design_qa_ended`, `member_left`, `reassigned`,
`handed_over`, `handed_back`, `could_not_start` (its worktree couldn't be made)
or `not_started` (the daemon stopped before it started) for the others. Screens
put the codes in words. An `active` run equipe paused itself carries `pause`
(`RunPause`): why, the engine, when it goes on and what the engine said.

## Starting

A start reserves the ticket on the daemon's loop and makes its worktree off it
(docs/ARCHITECTURE.md, "Execution: git worktrees"). The run is `active` from the
reservation, with its runtime session and lease, and carries the reason
`starting` until its session's first turn starts; the lease is renewed while the
worktree is made. A completion counts only for the run that still holds that
generation as reserved: one that comes after the run was cancelled, reassigned
or its member left starts nothing, and a worktree it made for nothing is
removed (with its branch when it cut one). A worktree that couldn't be made
stops the run (`could_not_start`) and sends the ticket back to the proposals with
the reason.

## Schema and migration

SQLite table `ticket_executions` keeps one row per ticket: state, runtime
session, operation key, generation, heartbeat and lease times, reason, update
time, and — for a stalled run — the checkout's abbreviated commit (`head`) and
its count of changed or untracked files (`dirty`). `execution_operations`
keeps every operation key ever used, so an approval replayed long after is
still recognised. Both tables are created with `IF NOT EXISTS`; added columns
are added with `ALTER TABLE` when missing.

An `in_progress` ticket from before runs were kept gets a row on the first
daemon start (generation 0, `migrated`) and is then treated like any run that
was active when the daemon stopped (below). Its ticket id, `TicketWork`,
branch, base, worktree, engine conversation, HEAD and every dirty or untracked
file remain untouched. Re-running it is a no-op.

## Lease and the stall check

The lease is 150 seconds. The minute tick renews it for every run whose turn
is running, then stalls the active runs whose lease ran out and whose session
has no running turn, nor a pause of equipe's (below). A running turn is never
stalled, however long a tool call takes; a turn that ended without a board call
and that equipe didn't carry on — one the person stopped, say — is stalled two
to three minutes later.

A stalled run's member lets go of the ticket in memory only: nothing is
committed, reset or cleaned, so HEAD and the uncommitted and untracked files
stay exactly as they were. The member is free for other work. The ticket stays
`in_progress` with its assignee, branch and worktree; its card shows the
commit and the dirty-file count, and the PM is told it can't recover the
ticket and must not open a new one.

## Turns that end without ending the work

The lease is the last net, not the way work is carried on. When a turn on a
ticket ends with no hand-in, wait, question, hand-back or design QA verdict,
equipe acts at once (`runtime/turn_end.rs`):

- **The engine failed the turn.** The run is paused (`pause.why = retrying`)
  and goes again by itself after 30 seconds, 2 minutes, 10 minutes, then every
  30 minutes — never at once. A turn that failed with nothing done (no tool
  call, no token) within 20 seconds is a fast failure. Three of a squad's
  tickets failing fast within five minutes open the squad's breaker
  (`squad_paused`): nothing new starts and nothing retries; the person is told
  once with what the engine said (secrets blanked); a test turn goes after two
  minutes (doubling to half an hour while it fails), at once when the person
  says anything to a member, the disk has room again or an engine signs in
  again. When it works, every paused run goes on. A conversation the engine
  can't resume ("No conversation found") is dropped, and the next try starts a
  fresh one with the ticket's brief, same branch and worktree. A lost sign-in
  is paused (`sign_in`) for `engine_auth`'s carry-on.
- **The engine reached its plan's limit** (its error, a limit line it ended on,
  or its plan read as out). The turn is kept as `limited` in the journal's
  turns, not completed. The run is paused (`engine_limit`) with the ticket as
  it is; the switching rules may move the member to the other engine, which
  hands the paused ticket over at once; otherwise it goes on in the same
  conversation when the engine is read with room, or at its reset.
- **No end to the work.** A member with the board's tools is resumed once in
  the same conversation with the ways to end a turn; ended the same way again,
  the ticket goes back among the proposals for the PM (`Ticket::handed_back`),
  with the member's last comment (or words) as the reason, and the PM is woken.
  A turn whose member handed its browser or a sign-in to the person, has a
  permission pending, or has messages waiting is left as it is.

A member hands a ticket back itself with `hand_back`: `waiting_on` (named
tickets) puts it back on the board depending on them, for the same member, in
the same worktree, once they are closed; `duplicate`, `not_my_role` and
`cannot` put it among the proposals with the note, and wake the PM to cancel,
reassign or reshape it. A turn the person stopped is left alone; its lease
still stalls it later. Each of these endings is counted in the squad's journal
(`turn_endings`: `no_end`, `nudged`, `handed_to_pm`, `handed_back`, `failed`,
`failed_fast`, `limited`, `squad_paused`), logged, and summed in the weekly
retro's facts.

## Recovery: the person's command only

`Command::Requeue { squad, ticket, operation, how }` is person-only
(`permissions::person_only`): the gateway refuses it from a member's own
process, there is no member tool for it, and the PM's `update_ticket` can't
move work in progress. `how` is one of:

- `resume` — the same member goes on, in a fresh runtime session;
- `requeue` — whoever of its role is free next starts a fresh run;
- `reassign` — the named member takes it over.

In one SQLite transaction the run becomes `queued` under a new generation, the
operation key is recorded, and the ticket goes back to `todo` for its new
owner. The ticket's note — the latest PM send-back, design QA or reopen reason
— its open blockers, branch and worktree are kept. Dispatch follows at once:
the owner, when free, starts the queued run under its operation and reserved
generation, and returned work (recoveries included) goes ahead of that
member's unstarted tickets. The same operation key twice is one recovery; the
person is told it happened already. A run that isn't stalled (or queued) is
never recovered: work in review stays in review, and a wait is the person's
`Resume` of the waiting ticket.

The recovered run's prompt opens with the reopen envelope as before — the
latest send-back or design QA reason first, marked as superseding any older
"handed in" message, then the open blockers — followed by a line saying the
last run stopped without a hand-in and this fresh session is on the same
branch and worktree with nothing reset.

An approved team change that adds members and names stalled tickets in
`unblocks` is also the person's approval to hand those tickets to the first new
member: owner and recovery in one transaction, for each ticket the new
member's role owns. The PM's proposal is refused before it reaches the person
when an unblock is owned by name by a member still on the team (`backend-2`):
a new member would never take it. Work owned by another role is left to the
board's rules.

## Restart

A run still `starting` when the daemon stopped never ran: before anything else,
its ticket goes back to `todo` (no assignee; owner, note, branch and work kept)
and the board starts it afresh. A recovery the person approved is `queued` again
under its operation and generation, to start exactly once; any other start is
`stopped` (`not_started`). What its worktree left half-made, tidy clears as
always.

No runtime session outlives the daemon process, so a start first stalls every
active run (`not_resumed`). Each member brought back on the ticket it ran —
its worktree still there, its engine able to resume here (Claude Code, or
Codex with its CLI) — carries it on under a new generation, as equipe always
did across restarts. What nothing carries on stays stalled for the person. An
approved recovery that was still `queued` survives the restart and starts
exactly once when its owner is free. The daemon never recreates a worktree,
resets a branch, cleans files or creates a recovery ticket.

## Screens

`Ticket::run_view(now, assignee_working)` gives every screen — board, sidebar,
ticket card, terminal board — the same reading: `Active`, `Paused` (in progress
but its member is idle, or its lease ran out), `Stalled` or `Queued`. A
stalled ticket's card offers Resume, Requeue and Reassign, each with a confirm
card showing the ticket, branch, worktree, commit, uncommitted file count and
latest blocker, and saying all of them are kept.

## Several runs under one member

A member is one identity — role, memory, settings, its conversation with the
person — however many tickets it works. Each ticket it takes runs in a session
of its own beside that conversation (`SessionInfo::run`, `Event::RunOpened`):
its own worktree, engine conversation, turn, steering, stop, requests to the
person and estimate. The run's session is the ticket's runtime session
(`TicketExecution::session`), so the run and its generation are one thing; a
single ticket goes the same way. The team lists the member once, its runs
under it. The PM's conversation lanes (`Session::lane`) are something else and
are never used for runs.

**Dispatch.** One pass hands out every ready ticket a member has room for
(`lanes::assign`): in the board's order, each to the member of its role with
the fewest tickets in hand (runs at work plus tickets waiting on the person),
ties to whoever joined first. A ticket owned by a member by name goes to that
member only, and waits for it while it can't take new work (its engine held,
a move to the other engine pending, a member brought in for a rush). Queued
work never adds a member: `propose_team_change` refuses an unasked add of a
role the team already holds; adding stays possible for a role nobody holds, or
when the person asks.

**Lane policy.** How many runs go at once comes from the plans and the disk,
never from the number of members (`lanes::member_limit`,
`lanes::engine_limit`): 3 runs per member and 8 per engine across the squad by
default; 4 and 10 under 25% of the tightest plan window used; 2 and 4 from
90%. Work keeps its full speed until then: the weekly reserve
(`crate::pacing`) keeps the PM and urgent work going once a plan runs low. With the disk low every member runs one at a time; from
critical, new code work waits altogether while tasks and actions still start.

**Tool calls.** Every run's board, desktop and phone servers are started with
`--run session:ticket:generation`, and each call carries it
(`ToolCall::run`). A call that changes something — a ticket, a question or a
permission to the person, an estimate, a hand-in, a fact remembered — is taken
only from the run that still holds its ticket's current generation (in memory
and in `ticket_executions`); from any other it is refused with nothing
changed, and the refusal goes on the ticket's record (`stale call`). Reading
the board still works. A call naming no run is the member's own conversation;
from an older board server with the member's conversation idle, it stands for
the member's one run when it has exactly one, and for none when it has
several.

**Ending.** A run closes when its ticket is put down — handed in, waiting,
accepted, cancelled, reassigned, handed over, stalled. What it still asked the
person is answered no; memos left for it go to its member's conversation.
Waiting, failing or being stopped touches only that run. The person stopping a
run (`Interrupt`, or `Close` on the run) stops its turn; the member stays.

**Restart.** Every ticket a member was running comes back as a run of its own,
each exactly once under a new generation, with what was said in it before.

**Screens.** A screen that says `runs` in its hello gets runs as they are. One
that doesn't (an older desktop or terminal, the phone) gets the squad
projected through `protocol::runs::Legacy`: runs are never listed as members;
a member with one run shows that ticket as its work; with several it shows
none and a line saying the screen must be updated to follow them — never one
run standing for the others.
