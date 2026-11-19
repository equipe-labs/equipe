# From a request to shipped work

A squad used to turn one thing the person asked for into a relay of tickets:
a spec ticket, its landing, a build ticket made for the frontend, a wait for
someone free, the build, the PM's review, a design QA ticket, the PM's
acceptance, then several "verify" tickets. Each hop was a new ticket waiting
in the queue, and the PM was woken for every hand-in. Little work came back
for quality; the time went into hand-offs and meta work. This is how a squad
runs a request now. The rules are in `crates/core/src/flow.rs` (pure, tested
alone); the daemon's side is `crates/core/src/runtime/flow.rs`; the numbers
come from `equipe flow` (`crates/core/src/lead_time.rs`).

## One request, one ticket, one driver

The ticket a member puts on the board in answer to the person (the request's
`asked` ticket, docs/REQUESTS.md) carries that request to production. The
member who made it **drives** it (`Ticket.flow.driver`); when the PM shapes
the ticket for someone else, that owner drives it.

Its spec, build and checks are a **checklist** on the ticket
(`Ticket.flow.steps`), not tickets of their own:

- a UI change: prototype or spec (designer) · build it from the prototype
  (frontend) · built screens checked against the prototype (equipe's precheck);
- other code: make the change · tests and checks pass;
- a task or an action: no checklist (it is one step).

`create_ticket` takes `steps` to give its own; `check_steps` ticks items off
(`done`) and adds items (`add`); the PM's `update_ticket` adds them too.

## The baton

When a holder's items are done and another role's are next, it calls
`pass_baton` (`to`, `note`, `done`): its changes are committed and the ticket
goes back on the board for `to` — branch and worktree kept, no new ticket, no
PM step — handed out ahead of new work. The next holder starts a
conversation of its own in the same worktree, told who passed it what
(`Baton from …`) and its checklist. With `back`, a teammate is pulled in for
one step and the baton returns to whoever lent it.

The board shows who holds it now, from whom, who drives it, and the checklist
ticked so far. Each pass is a `baton` event on the ticket's record.

## Blocked on work that doesn't exist yet

A baton is for work that exists. When the next part needs another role's or
project's work that nobody made — an API or endpoint, a contract, a
permission, data — the holder names it: `blocked_on: { role, what, project? }`
on `hand_back` or `pass_baton` (`crate::blocked_on`). equipe finds that ticket
on the board (one the blocked ticket waits on already, one made for it
before, or one of its request of that role that says the same) or makes it
for the role, under the same request, at least high priority; the blocked
ticket waits on it (`depends_on`, a `blocked on` event) and goes back to its
member, in the same worktree, once it lands. Nobody passes it meanwhile.

Members who say it only in prose are caught by their notes: the second
hand-back or baton on one ticket whose note says something it needs is
missing (`blocked.missing_terms` and `blocked.dependency_terms`, every
language's) isn't passed on. The ticket goes to the PM with one card, once:
make the missing ticket — `update_ticket` with `blocked_on` — or reshape or
cancel it. Before, a frontend ticket that needed a missing server endpoint
went around frontend, QA and the PM thirteen times in 1h41m.

## A UI change is one slice

A code ticket that answers the person, made by or for the designer, is one
**slice**: the designer prototypes or specs on the ticket's branch, passes
the baton, and the frontend builds it in the same worktree. When it is
handed in, equipe's precheck compares the built screens with the prototype
and alone decides, as for a prototype the person picked (docs/DESIGN-QA.md):
a designer looks only at what it fails. Its check is recorded as equipe's own
(a design QA record closed by the precheck, never news); when the precheck
can't run, a designer checks it as before. A designer's own design QA turn
stays for changes nobody asked for.

## Members land their own work

Where the PM accepts the team's work (`pm_accepts`, the default) and the
squad lets members land (`self_land`, on by default — `change_setting
self_land off` keeps every acceptance with the PM), work that is ready lands
by its own member, through the PM's own acceptance path: a look for
conflicts with main, the project's checks on the merge (`.equipe/checks`),
the landing queue. A conflict or a failed check sends it back as before.

It doesn't when the squad's autonomy policy keeps merges for the person
(`merge: ask`, docs/AUTONOMY.md), or when the importance gate holds it (`person_gate::weigh`): an
action in the world, a deploy, production data, permissions or security,
money, legal matters, more than a dozen screen files changed — or the PM's
own code. Those wake the PM as before, which puts to the person what needs
them.

The PM reviews after the fact: once the first landed work has waited 30
minutes, or five are in, it gets one digest of what landed, each with what
changed; `revert_landed` (with why) puts a revert ticket at the front of the
member's queue, and a follow-up ticket for the member adds what is missing.
The person sees the same work under **Decided for you** ("Landed by …"),
with **Revert** on each.

## No tickets for meta work

`create_ticket` turns away, with the reason:

- a check (`label: check`) or polish (S3/S4) of the ticket the member is at
  work on — it goes into that ticket's checklist, or out of scope into a
  `comment` or the design backlog;
- a check of another ticket under way (named, or depended on) — into that
  ticket's checklist;
- the next part of a request made while working its ticket (the build after a
  spec) — it goes on the same ticket by `pass_baton`, unless `separate_why`
  says why it is work of its own (another repository, nothing on the branch).

An S1/S2 finding is always a ticket. A designer's S3/S4 finding goes into the
design backlog (an idea nothing starts) rather than a polish ticket, and the
daily design review's polish goes into one backlog note the same way.

## Lanes fit the machine

A member still runs several tickets at once in lanes; no members are added
for a queue. Its lane cap now also follows the machine, as the daemon's
minute look at it reads it (docs/LOAD.md, which holds new work back
altogether once the machine is overloaded): short of that, the 1-minute load
over its cores at 1.0 takes a lane away, at 1.5 halves the cap, at 2.0 leaves
one, and less than a tenth of memory free leaves one. Two lanes of one member never work on places that overlap (the paths
their tickets name). When the ready queue is longer than twice what the
squad runs at once, a member with a lane at work takes only work already
under way — a baton passed, work sent back — so the queue finishes before it
starts.

## Measuring it: `equipe flow`

`equipe flow --since 48h [--squad …] [--db …] [--json]` reports, for the
window: requests from the person; request → first work, → first merge,
→ production (a deploy ticket done, else the last of its work finished),
median and p90; tickets and hand-offs per request; ticket made → first
assigned and → merged; the share of meta tickets and of their tokens; who
made the tickets; baton passes and self-landed work; the slowest requests.
A journal from before requests were kept has them worked out from the
person's words and when tickets were made, and the report says which.

A request answered with nothing to ship — a decision or an instruction
("go with B", "skip that test and ship it") whose tickets all finished with
nothing merged, waiting to merge or deployed — is *decided*: counted apart
("Answered or decided, nothing to ship"), left out of request → production
and of the slowest list, and its status line says "answered — nothing to
ship" rather than suggesting it waits for production.

### Before

A read-only copy of a production squad's journal, 48 hours to 2026-11-18
(requests worked out, as that journal keeps none):

| | median | p90 |
|---|---|---|
| request → first work | 9m | 3h57m |
| request → first merge | 1h23m | 7h10m |
| request → production | 1h55m | 17h33m |
| ticket made → first assigned | 23m | 4h23m |
| ticket made → merged | 2h55m | 9h58m |
| tickets per request | 2 | 12 |
| hand-offs per request | 0 | 4 |

65 requests; 977 tickets made, 48.2% of them meta, holding 28.7% of tokens.

### After

Once a squad runs a release with this, run `equipe flow --since 48h` over
the 48 hours that follow it (and `--since 7d` after a week) and compare the
same rows. What should move: tickets per request and the meta share down,
baton passes and self-landed work up, request → first merge and
→ production down. Send-backs (`equipe usage`) and S1/S2 fix tickets should
not rise.
