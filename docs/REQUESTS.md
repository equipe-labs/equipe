# The person's requests

One thing the person asks a member for fans out into many tickets: a spec,
its build, a design QA, a licence check, a phone check waiting on them. They
shouldn't have to ask after each one. A **request** keeps them together, says
in one line where all of it stands, tells the person when something lands or
needs them, and lets every member answer "how's X going?" at once.

The rules are in `crates/core/src/requests.rs` (pure, tested alone); the
runtime's side is `crates/core/src/runtime/requests.rs`; the wire types are
`equipe_protocol::requests`; the desktop draws them in
`crates/desktop/src/views/requests.rs`.

## When a request is made

- The person asks a member for something in a conversation (its own or a
  lane of it, not beside a ticket's run): each message is a request of its
  own from then on — so two things asked minutes apart are two requests,
  never one. Its title is the first line of their words (joined by the next
  ones while that line is too short to say much: "Hi!", "Also:"). A screen
  lists it once a ticket is with it, or once it is uncovered (below).
- The person puts a ticket on the board themselves: a request of its own, its
  news told in the PM's conversation.
- A message that only asks after a request ("how's the input box going?"), or
  that names a ticket already on the board ("T-12: …", "merge T-3"), is not a
  new one.

## Which request a ticket serves

A ticket a member puts on the board from its own conversation, not at work on
a ticket, serves the person's request there that it answers:

- named: `request` on `create_ticket` (`3` or `R-3`; `0` for none of them);
- not named, with exactly one of the person's requests that it could answer —
  open, asked in a conversation of that member, not all done, and asked in
  the last 20 minutes or still with no ticket covering it — that one;
- not named, with more than one: **refused**, before anything is made, with
  each request in one line (`R-3 “Login and logout show an error” (no ticket
  yet) · asked 13:26`), so the member names it. A ticket is never put with
  whichever request was asked last.

A ticket made while working a request's ticket joins that request (`made`),
whatever is open. The reply names the request in the person's words, so a
wrong one shows at once; the PM moves a ticket with `update_ticket`'s
`request` (`0`: with none), and what came of it — made while working it, its
follow-ups, its checks — moves with it.

## How tickets join it

| Link | When |
|---|---|
| `asked` | made in answer to the person's words, named or the one open request (or by the person) |
| `made` | filed by a member while it worked a ticket of the request (a run's `create_ticket`) |
| `follow_up` | a design follow-up (`design_follow_ups`) of a ticket of the request |
| `check` | the design QA of a ticket of the request: shown in the timeline, never in the status line, never news |
| `linked` | a member named it while at work on another request's ticket, or the PM's `update_ticket` with `request` (any status) |

A ticket serves one request at most, and what came of it follows it when it
moves. Kept ideas never join one.

## Covered, or not

Only work **covers** a request: a ticket that serves it that isn't equipe's
own check (a `check` link, or anything that came of one), a design QA ticket,
or a meta ticket (labelled `check`). A request no ticket covers ten minutes
after it was asked (`COVER_WAIT`) is **uncovered**:

- the PM is told once, in one message listing each in the person's words, to
  put its work on the board or close it with why — and every PM turn lists it
  until one covers it (a member's own turns list those asked there);
- the person sees it listed, its line starting "no ticket serves it yet — the
  PM is told";
- it never closes by itself.

## Closing honestly

A request closes by itself only when a ticket covers it, every part is
finished and nothing waits on the person — told once as "all done". Another
request's tickets never close it, nor do equipe's checks. New work joining a
closed request opens it again (a check joining doesn't), and its "all done" is
told anew.

`close_request` (the PM, or the member it was asked of) closes one that asks
for no work — thanks, a question answered — or no longer does, with why in a
few words. The person is told in the conversation they asked in ("Your
request “…”: closed: answered in chat"); one with tickets stays listed with
that line. Its open tickets stay on the board unless the member cancels them.


## The status line

Each part reads as how far its open tickets got — stopped (its run stalled),
someone on it, in review, up next — and, once none is open, as what was done:
live in main since a time, accepted but not on main yet, done, dropped. What
waits on the person (a ticket put down for them, or one with a question open
to them) is said apart, with a count. When every part is finished and nothing
waits, it is "all done".

> Spec the Tinumi composer — live in main since 07:58 · Left menu — frontend
> is on it · 2 things wait for you (Check the new composer on r…, Decide if the
> layout needs…)

## News, once each

A ticket's state is news when it **waits on the person**, its run **stopped**,
it is **done**, or its work is **on main** — and the request is news when it is
**all done**. Being worked on, handed in, sent back and handed in again, and
everything about equipe's own checks are not. Each of those marks is told once
per ticket (kept in the request's `told`), so a retry or a second wait on the
same ticket says nothing new.

The first change starts a two-minute wait (`SETTLE`); the minute tick then
tells everything that moved meanwhile in **one** line — the request's status —
as a notice in the conversation it was asked in (else its member's own), and
an `Event::RequestNews` the screens turn into a notification (as a reply is).

## What members are told

A member's turn in a conversation starts with the person's open requests there,
each in one line — its number, its name in their words, its tickets (or "no
ticket yet"), when it was asked, its status line — with the instruction to
answer "how is it going?" from it at once, to give each a ticket that serves
it, and to close what asks for no work. The PM's turns add the requests asked
of other members that no ticket covers yet. While the member is busy, a message that asks after
one of them (by ticket or R-number, or by a term of its words or its parts'
titles, in a question) is answered at once by equipe from the board, marked
as equipe's answer; the member still reads the message at its next step.

## Stopping a kind of work

"Stop this" on a request (two presses: it cancels tickets), or the person's
words to a member, who calls `stop_work` (refused unless the person spoke to
that member in the last 20 minutes), keeps a **stop** for the squad
(`crates/core/src/stops.rs`, `runtime/stops.rs`):

- the open tickets of its chain — the request's tickets, or a ticket's part
  and its design follow-ups — are cancelled;
- a new ticket is turned away when it comes of the chain (made while working
  one, a follow-up of one), serves the stopped request, has every word of one
  of its terms in its title (taken from what most of the chain's titles share,
  or given), or names one of its paths. The member is told why and to ask the
  person first. A design review files no fix or polish for it either.
- a member's ticket made in answer to the person's own words naming that work
  again, after the stop, goes on the board;
- every member's turns start with the stops in force; the requests view lists
  them ("Stopped: gallery polish · since 11-18") with **Lift**.

## Settled questions

A question put to the person about a ticket that closes — done, cancelled,
merged — is withdrawn when it closes, with one line in the asking member's
conversation (`runtime/settled.rs`). Cards kept from before are swept when the
squad's questions are first read.

## Only what matters reaches the person

Where the PM accepts the team's work, `propose_accept` puts a card in front of
the person only when the work is weighty (`crates/core/src/person_gate.rs`):
it acts in the world (an `action` ticket), can't be taken back (a deploy,
infrastructure), touches production data, permissions or security, money,
legal matters, or is a big visible change — by what the ticket is, the words
of its title, or the PM's own `why`. Anything else gets no card: the PM is
told to decide it itself (`accept_work`, or a send-back), and it goes into the
person's daily digest, **Decided for you** under Your requests (what the PM
accepted or sent back in the last day, each with what changed). A card that
does go up says why ("Needs your yes: “Add the roles table” (production
data)"). The person's own words read by a screen always raise their card.

Every card names each ticket by its title, and the accept card adds one plain
line on what changes for people (the first sentence of the hand-in, in the
person's language) — never a bare list of numbers.

## What the person asked for doesn't queue behind internal work

- **First in the queue.** Ready tickets of open requests (asked, made for
  them, linked — not equipe's checks nor design follow-ups) are handed out
  before the rest of the same members' work (`Store::serving`, in
  `dispatch_to`'s `first`).
- **No chaining without a reason.** A ticket of a request — being made in
  answer to the person, made while working one, or already in one — may not
  wait (`depends_on`, on `create_ticket` or `update_ticket`) on open work
  outside that request unless `depends_why` says why. Any dependency whose
  named places (`design/…`, `src/…`) overlap nothing with the ticket's is
  pointed out in the reply.
- **A member unblocks its own.** `unblock` takes dependencies off the
  member's own ticket that hasn't started, with why (logged on the ticket, the
  PM told). On a ticket that isn't its own or has started, it asks the PM
  instead — a `[Unblock · member]` message the PM takes up at its next turn,
  to settle first.
