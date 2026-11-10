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
`accepted`, `cancelled`, `design_qa_ended`, `member_left`, `reassigned` or
`handed_over` for the others. Screens put the codes in words.

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
has no running turn. A running turn is never stalled, however long a tool
call takes; a turn that ended without a board call is stalled two to three
minutes later.

A stalled run's member lets go of the ticket in memory only: nothing is
committed, reset or cleaned, so HEAD and the uncommitted and untracked files
stay exactly as they were. The member is free for other work. The ticket stays
`in_progress` with its assignee, branch and worktree; its card shows the
commit and the dirty-file count, and the PM is told it can't recover the
ticket and must not open a new one.

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
