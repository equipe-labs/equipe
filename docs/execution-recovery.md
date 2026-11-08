# Ticket execution recovery

Ticket workflow (`todo`, `in_progress`, `waiting`, `review`, …) describes the
work product. It is deliberately separate from the runtime execution state
(`active`, `waiting`, `stalled`, `stopped`). A ticket may therefore remain
`in_progress` while its last process is `stalled`; recovery never creates a
replacement ticket.

## Root cause and state transitions

Previously, `in_progress` and an open member session were treated as proof that
an engine was running. A process could disappear without a terminal board call,
leaving the ticket permanently assigned, or a daemon restart could blindly
resume work whose process no longer existed.

Each start now owns a durable `(ticket, generation, session, operation)` lease.
It enters `active`, is renewed while that runtime turn is observable, and ends
as `waiting` when the person is needed or `stopped` when the work is handed in,
accepted, cancelled, reassigned, or released. Recovery changes `active` to
`stalled` only when its runtime session is absent (or its lease has expired and
is absent). A live session always wins over wall-clock expiry, avoiding a false
stale decision during a long tool call.

## Schema and migration

SQLite table `ticket_executions` stores the state, runtime session, unique
operation key, monotonically increasing generation, heartbeat and lease times,
reason, and update time. It has one row per ticket and a unique operation key.
The table and lease index are created with `IF NOT EXISTS`.

On first open, an older `in_progress` ticket without an execution row is
classified as `stalled` with generation zero. Its ticket ID, `TicketWork`,
branch, base, worktree, engine conversation, HEAD, and every dirty or untracked
file remain untouched. Re-running the migration is a no-op.

## Lease selection and fencing

The runtime uses a 150-second lease and renews it on the minute tick for turns
that are still present and working. This leaves more than one missed tick of
clock/scheduling margin. Heartbeats update only an exact
`ticket + generation + session` active row, so an old process cannot revive or
extend a newer generation. A valid active lease prevents a competing scheduler
from acquiring the same ticket.

## Approved requeue and restart policy

`Requeue` is a person-only protocol command. Its operation key is reserved in
the same SQLite transaction that returns the existing ticket to `todo` (and,
for an approved hand-off, can set its owner). Dispatch immediately binds that
reservation to a fresh runtime session; Codex-to-Codex is still a new runtime
session even if the saved engine conversation is reusable. Replaying the same
operation returns without creating another generation or execution.

After a daemon restart there are no surviving in-process runtime sessions, so
their active executions become `stalled`. The daemon does not recreate a
worktree, reset a branch, clean files, or manufacture a recovery ticket. The
person-approved requeue resumes the same ticket and lets the normal worktree
path reopen the saved checkout. If the checkout is actually missing, normal
start validation reports that failure instead of silently discarding work.

The reopen prompt continues to use the ticket's latest `Reopened`/`Resumed` or
design-QA note and the ticket's remaining blockers. Those current fields take
precedence over older hand-in conversation text.
