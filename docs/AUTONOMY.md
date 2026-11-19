# Autonomy policy

A squad should keep building with as little of the person's time as possible,
and never without them on what is theirs. The autonomy policy is how the person
says, once, which is which. It is a layer other parts of equipe plug into: the
importance gate, the daily digest and "stop this kind of work" rules read what
it decides; it does not replace them.

Code: `crates/protocol/src/autonomy.rs` (types, levels, the floor, pure rules),
`crates/core/src/autonomy.rs` (matching, classification, the squad's store),
`crates/core/src/runtime/autonomy.rs` (the daemon at work),
`crates/desktop/src/views/settings_autonomy.rs` (Settings › Permissions ›
Autonomy and the "Picked for you" card), `crates/cli/src/autonomy.rs`
(`equipe squad autonomy`).

## Levels

Each kind of thing gets one of three levels:

| Level | What happens |
| --- | --- |
| **Ask** | The person decides first. |
| **Decide and tell** | The member goes ahead after a short window; the person gets a one-line note and can change it. |
| **Just do** | The member decides; it is logged. |

| Kind | Default | Notes |
| --- | --- | --- |
| Design and product choices between options a member made | Decide and tell (30 min) | A designer's choice, a pictured or tried one, or any choice whose every question has the member's `recommended` option (and says nothing of live data). The member carries on with its recommendation at once — its ticket is never put down for the choice — and the option is taken when the window passes unless the person picks otherwise. Low-priority tickets don't wait. A design choice between options with no recommendation is sent back once to mark one (a product choice without one is asked); a taste board, an open question, or one whose words name the floor is always asked. `block_on_person` refuses a need that is a question (it ends on a question mark) and points to `ask_decision`. |
| Merging reviewed work | Just do | The same switch as "the PM accepts the team's work" (`pm_accepts`). The importance gate (`person_gate`) reads this policy: small work is the PM's to accept; work weighty for a reason on the floor (outward, irreversible, security, money, legal) or a big visible change always goes to the person on a card, and work touching production data does unless the person set "writing to live data" to just-do (`Why::pm_may`). |
| Internal and process questions | Just do | `ask_decision` refuses a set whose every question the member marked `kind: internal` and tells it to decide and note its choice on the ticket. |
| Writing to live data | Ask | Open to standing allowances (below). Set to decide-and-tell or just-do, it goes ahead with a one-line note. A schema migration that only adds (CREATE TABLE/INDEX, ADD COLUMN nullable or with a default — read from its SQL by `crate::migrations`) isn't a live-data write to ask about: it lands like code, and the PM's digest of landed work says so; an `ask_person` that names only such files is answered "not asked". One that drops, deletes, updates existing rows, changes a type or renames still needs the person. |
| Production deploys after a landing | Just do | Running a project's declared go-live command (`.equipe/deploy.json`, or one the PM confirmed with `set_deploy`) once its work landed and main was pushed ([USAGE.md](USAGE.md#going-live)). Set to ask, nothing deploys on its own: the PM is told the work is on main, not live, and asks the person first. Decide-and-tell deploys at once, like just-do: what went live is told either way. A deploy ticket, or work whose files deploy (a workflow, infrastructure), still goes to the person through the importance gate. |

### The floor

These always ask, whatever the policy, whatever a member says a request is:
**sign-ins and credentials, payments and money, permanent deletion, sending or
publishing outside, security settings, legal matters**. `Policy::set` refuses
anything but "ask" for them; a standing allowance can't be granted for them
(`Kind::allowable`) and never matches one (`autonomy::covering`).

A request is put on the floor by what it is, not by what it claims:

- an engine's permission prompt by its tool and command — outward commands
  (`git push`, `gh`, `vercel`…), credential paths, deleting commands
  (`autonomy::engine_kind`);
- a member's own request (`ask_person`) by the floor kind it declares, and by
  the words of its `action` in every language equipe speaks
  (`match.floor.*` in `crates/protocol/locales`, `Kind::floor_named`). A request
  that says "production data" but asks to email someone is "outside".
- a question set by the words of its questions and options.

Word matching errs towards asking: a false hit costs one card.

### Who sets it

Only the person, by their own action: the Settings screen, the command line
(`equipe squad autonomy`, which goes through the daemon as their own command,
so a member's shell can't send it), or a tap on a card. Every command here is
person-only (`permissions::person_only`). The PM may *propose* a change with
`change_setting` (`setting: autonomy`): one that only narrows is done at once,
one that widens is put to the person on an approval card. A member's or the
PM's words never count as the person's approval.

```
equipe squad autonomy                          # show
equipe squad autonomy set design just_do
equipe squad autonomy set production_data decide_and_tell
equipe squad autonomy set deploy ask
equipe squad autonomy window 60                # minutes; 0: at once
equipe squad autonomy revoke 3                 # a standing allowance
```

## Standing allowances

A request's card offers **Allow for this kind** next to Allow Once, with the
scope spelled out in plain words and an expiry (1, 7 or 30 days; 7 by default,
90 at most). It appears only where something sound can be generalised:

- an engine prompt: the rule "always allow" would keep (`Bash(npm test:*)`),
  now with an expiry and a log;
- a member's write to live data that offers a `standing` scope: a system
  (`ERP`), a marker every such record carries in its title (`[test]`, at least
  three characters), the ops, and optionally how many days the member may
  delete what it made. For example: *test records titled `[test]` in the ERP
  may be created, and deleted by the member that made them within 7 days.*

A later request is allowed at once when a live allowance of its kind covers
it: on its system, carrying its marker, doing one of its ops, naming nothing on
the floor. A delete is covered only for the same member, within the days, after
it created something under that allowance — cleaning up its own test records is
not "permanent deletion" of the person's data; any other delete is. Each use is
logged (the allowance's use count, the squad's use log, the ticket's history)
and a member's own request posts a one-line note in its conversation. The
person can list and revoke allowances in Settings or on the command line.

The store lives in the squad's journal (`meta.autonomy`): the policy, the
allowances, the latest 500 uses, the automatic picks of the last week, and the
choices waiting for their window.

## Plain words first

`ask_person` takes a `summary` — what will change, who sees it, how it is
undone — in the person's language, and a `kind`. The card leads with those
three lines; the member's technical text (and its reason) folds under "Show
the details".

## Batching: nobody idles on one ask

`ask_person` no longer holds the member's turn for up to 30 minutes (26 of the
112 permission requests in the data below timed out that way, the member idle
the whole time and then told "declined"). The member waits three minutes for a
quick answer — a yes took four minutes on average, often under one — and is
then let go: the card stays up, the member carries on with work that doesn't
depend on it, and the answer comes as a message. If the member ends its turn
with nothing else to do, its ticket waits on the person (the request is listed
among what it waits on) and goes back to it with the answer. An unanswered
request is declined after a day. Everything still waiting sits in the one queue
the importance gate sorts by impact; a request its member no longer waits on
doesn't count as an idle member there.

## Waiting on you: one list, never out of sight

Everything that waits on the person is gathered into one list
(`importance::waiting_list`): tickets put down for them with their checklist's
open items, runs to bring back, cards to accept, questions, permissions,
sign-ins and secrets. Each says in plain words what to do (a waiting ticket's
one-line ask, not the member's title), how long it has waited, what it holds
up, and which of the person's requests it holds up. The most blocking comes
first, then the longest waiting.

- **The person** reads it in one line in the PM's conversation — "Waiting on
  you (9): Sign in to AWS (5h) · Delete the test ERP draft (3h) · …" — when
  something new joins it, at most every half hour (`TELL_EVERY`); the Status
  screen lists it too, with the same details. On the desktop, Needs you holds
  every project's and the PM desk's whatever project is in focus, each waiting
  ticket with its open checklist items, how long it has waited and the request
  it holds up; its count shows on the squad's row in the sidebar's foot, a
  click from the list.
- **The PM** finds all of it (up to eight, then a count) at the start of its
  turns, whether or not it holds other work up. Anything waiting three hours
  or more (`STALE_AFTER`) is marked stale, and is handed to the PM once so it
  brings it up with the person in plain words — or finds a way around it.

- **A card that is no longer needed is taken away**, never left up with words
  to ignore it (`withdraw_card`, `runtime/withdraw.rs`). A member takes back
  its own cards; the PM any open card on the squad — a secret or key request,
  a sign-in, an `ask_person`, a decision, its own proposal to accept — with a
  one-line reason the person reads where the card was. The card closes on
  every screen as if they had closed it (it leaves the composer and Needs
  you), and the member that raised it is told it was withdrawn, not that the
  person said no. equipe does this by itself for a ticket's secret, sign-in
  and `ask_person` cards when the ticket is re-planned (the PM changes its
  body), brought back after a stall, sent back, or closed. When a PM reply
  says a card is withdrawn, not needed or to be ignored while it is still up
  (`match.card_words` and `match.withdraw_words`, in every language), its next
  turn is reminded once to take it away.

## Decide and tell, step by step

1. A member asks a design choice with a recommended option.
2. Within the window the person may answer as ever; the card says nothing has
   been decided yet.
3. When the window passes (checked every minute; survives a restart, since the
   due time is kept in the journal) the recommendation is taken: the card
   closes, the member gets the answers ("taken under the squad's policy… they
   may still change it"), the ticket's history and the week's waits record it
   (`outcome: auto-picked`), the PM is briefed, and the person gets a one-line
   note in the member's conversation and a **Picked for you** card over the
   composer: *designer picked «B» (why) · Change · OK*.
4. **Change** opens the same question again for the person. Their answer goes
   to the member as "the person changed the choice you took on your own…" (or
   "kept it"). Picks can be changed for a week.

An automatic pick is never recorded as the person's taste; only their own
answer is.

## Why these defaults

From one production squad's 14 days (3–18 Nov 2026), read from its journal's
`waits` table: 387 asks, about 28 a day on active days, 89 at the peak.

| Kind of ask | Count | Outcome |
| --- | --- | --- |
| Decisions (`ask_decision`) | 275 | 252 answered, avg. wait 3.7 h; 23 closed unanswered |
| — of which design choices between options | ~101 (+ many more among scope questions) | 99 answered |
| — internal/process (ticket titles rejected, duplicates, which follow-up first) | ~27 | all answered, all trivial |
| Permission requests | 112 | 69 allowed, 17 declined, **26 timed out after 30 min** |
| — live data writes and live tests (ERP test records, migrations, deploys) | ~40 | mixed; often the same kind of test record again |
| — sending/publishing outside (Slack DMs, public issues) | ~19 | mixed |
| — deletion, credentials, money, security | ~31 | mostly allowed, but the person's to decide |
| — merges ("Merge T-… into main and push") | 9 | all allowed |

So: design choices are the largest single kind and the person almost always
takes one of the offered options, but waits hours — decide-and-tell with a
30-minute window keeps the designer moving while leaving the person the last
word. Merges were always allowed: just do. Internal questions were all
trivial: just do. Live data is mixed and consequential: ask, with standing
allowances for the repetitive part (test records). The floor kinds stay asked.

## Testing

`cargo test -p equipe-protocol autonomy` (levels, floor, words, recommended
answers, due times), `cargo test -p equipe-core --lib autonomy` (allowance
matching, expiry, revoke, deletes of one's own, engine and request kinds,
question kinds, the store), `cargo test -p equipe autonomy` (the command line),
`cargo test -p equipe-desktop autonomy approvals` (the card and the dock).
