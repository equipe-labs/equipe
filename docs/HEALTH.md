# Squad health: waste stops and self-watch

A squad shouldn't need someone outside it checking every hour whether it is
stuck or burning tokens. equipe watches for that itself, and tells the PM (a
line on its brief) and the person (news in the PM's conversation) in plain
words, naming the tickets.

The rules live in `crates/core/src/spend.rs` (waste stops) and
`crates/core/src/health.rs` (self-watch), as pure functions tested on fixture
journals; `crates/core/src/runtime/guards.rs` carries them out in the daemon.

## Waste stops

### A ticket's budget

Every turn's tokens are kept on its ticket (the journal's `turns` table, the
engine's own sum: input, cached input and output). Once one ticket's turns have
used more than the squad allows a ticket, **nothing more runs on it after the
turn under way**:

- a run that is working on it stops at the end of its turn, and the ticket is
  handed back to the PM — a proposal again, its branch and worktree kept;
- a ticket to do that already cost its budget (sent back, passed on, put back)
  doesn't start; it goes to the PM the same way.

The PM is woken with a card: what the ticket cost, in how many turns, how many
times it changed hands and was sent back, and its last hand-in or hand-off note.
It chooses one of three ways on:

| choice | how |
|---|---|
| go on | `update_ticket` with status `todo`: the ticket gets another budget, counted from what it had spent when it stopped |
| narrow it | a new, smaller ticket for what is really left, and this one cancelled |
| drop it | `update_ticket` with status `cancelled` |

The ticket's record says `budget stop`, with the tokens spent then, so the next
budget is counted from there.

The same member taking turn after turn on one ticket without a new commit on
its branch (6 in a row by default) stops it the same way, before its budget.

Defaults: **30M tokens per ticket**, **6 repeated turns**. Per squad:

```sh
equipe health budget          # what it is
equipe health budget 50M      # tokens per ticket (k, M, B)
equipe health budget off      # no budget per ticket
```

### The squad's spend rate

Every ten minutes, the last hour's tokens are compared with the squad's own
hourly median over the week before (hours that spent nothing are left out, so
nights don't drag it down; a week with fewer than 24 such hours has no median
yet). Past **twice the median** (and at least 2M tokens), the PM and the person
get one line naming the three tickets that spent most that hour — at most once
an hour, and only while it stays high.

### Plan headroom

When **every signed-in engine's weekly window** is 90% used or more, the PM and
the person are told once — and once more at 95% — with each plan's use and
reset, what still has room, how many tickets run now, and the low-priority
tickets worth putting off until the reset. The readings are the ones equipe
already keeps for switching engines (`runtime::engines`); an engine never read
means nothing is said. Once the weeks drop below 90% again, the next fill is
told again.

## Self-watch

Every ten minutes, off the daemon's work, the squad looks at itself. Each
finding is told once, and again only after it went away or when something new
joins it (a new ticket an idle squad waits on, a new thing waiting on the
person). What was told is kept in the journal, so a restart doesn't repeat it.

| finding | when |
|---|---|
| stalled ticket | in progress, no turn for 2 h, and its member is free (a stopped run is on the person's waiting list instead) |
| idle squad | nothing has run for 30 min while tickets wait to be done — with what they wait on: the tickets, not to do themselves, that hold them through their dependencies (waiting on the person, in review, a proposal…), and how many were ready but didn't start. The PM is woken for this one: nothing else would read its brief |
| main not pushed | the project's main has had commits origin's main lacks for 10 min |
| main not live | the project deploys some way equipe can see (its own go-live command, production deployments or a deploy workflow on GitHub), and origin's main has had commits past what is live for 30 min |
| waiting on the person | something on the person's waiting list (a ticket waiting on them, a card to confirm, a decision, a permission, a sign-in) has waited 2 h. That list already tells the person and the PM by itself (`importance`, "waiting on you"), so the self-watch doesn't tell it again; `equipe health` shows it |
| going in circles | one ticket was handed back, sent back, or reopened 3 times within a day |

What is live is read through a small trait (`health::LiveState`): the last
commit a project's own go-live command deployed (`crates/core/src/go_live.rs`),
else the deploy watch (`crates/core/src/deploys.rs`).

### `equipe health`

```sh
equipe health                 # this folder's squad
equipe health --squad acme --json
```

prints the same findings, worked out from the squad's journal and its projects'
git without asking the daemon (what runs now is read from the runs' leases), and
the squad's budget per ticket. `--json` gives each finding with its kind and
numbers, for scripts.
