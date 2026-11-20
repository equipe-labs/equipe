# Growth

equipe's growth loop reads a product's data by itself, finds what to improve,
recommends next actions as cards the person answers, ships the accepted ones
and then measures whether they worked.

```
weekly run ─▶ evidence (numbers, queries, days) ─▶ PM: ≤ 5 cards ─▶ person: accept · later · no
                                                                       │
             worked / no change / worse ◀─ measured at +7 and +14 days ◀─ live ◀─ ticket (+ request)
```

Code: `crates/core/src/growth.rs` (the rules, tested alone),
`crates/core/src/runtime/growth.rs` (gathering, cards, follow-ups),
`crates/protocol/src/growth.rs` (what screens read).

While a squad's [sprint loop](SPRINTS.md) runs, the sprint review reads these
numbers and the weekly run doesn't start; cards already open carry on.

## A run

Once a week — Mondays from 09:00, the daemon machine's local time — for a
squad with a PostHog connection or a project that declares
`.equipe/growth.json`, and whenever the person asks:

```sh
equipe growth run --squad acme
equipe growth list --squad acme          # the cards and their outcomes
```

A run has two parts:

1. **A deterministic pass**, off the daemon's loop, that counts. Each number
   becomes an **evidence** item: what was counted, its value beside the period
   before, the days (`from`–`to`), the query that read it and where it came from.
   - The squad's PostHog connection, through [ad-hoc read-only queries](METRICS.md#metricshogql)
     (the key stays in the daemon): each key event's count, each funnel's
     step-to-step and whole conversion, the first funnel per audience — the
     last 7 whole days beside the 7 before. At most 8 asks (16 queries) a run,
     within the connection's hourly budget.
   - The numbers equipe already keeps (`metrics` snapshots of PostHog, GitHub,
     App Store Connect): each daily count, this week beside last.
   - Error signals equipe sees: its signals that failed in the week (the share
     of readings that passed), the project checks it ran on accepted work (the
     share that passed), and the health checks a project declares.
2. **One PM turn.** The evidence goes to the PM in one message; the PM
   proposes at most N cards (default 5) with `propose_growth`, or none.

Nothing found, nothing handed over: no numbers, no cards.

## Cards

Each card carries:

| field | what |
|---|---|
| `evidence` | the run's evidence it stands on, by id (`E1`…): numbers, query, days |
| `change` | the proposed change |
| `metric`, `expected` | the metric it should move (one of its evidence's) and the effect expected |
| `size` | S, M or L |
| `risk` | low, medium or high |

`propose_growth` is checked in code (`growth::cards`), all cards or none:

- **No numbers, no card.** A card must cite evidence of the latest run (at
  most 7 days old) by id, each with a value. An unknown id, no ids, or a metric
  that isn't one of its evidence's refuses the call.
- At most N cards open at once (waiting on the person, or "later").
- Nothing like a card the person said no to in the last 60 days, nor like one
  already open (same metric, mostly the same words).
- **Privacy:** cards carry aggregate counts only — the run's queries count,
  they never list people. A card naming an email address or a phone number is
  refused, unless the project lists that field (`email`, `phone`) in
  `allowedFields`.

The cards go to the person on **one decision card** — the same card every
decision uses — one question each, with its numbers, and three answers:

- **Accept** — equipe puts a ticket on the board (`Growth G-3: …`), which
  serves a request of its own, followed like any request the person makes;
  the PM gives it an owner.
- **Later** — asked again with the next run's cards (for up to 30 days).
- **No** — remembered: not proposed again for 60 days.

## Measuring

An accepted card is **live** when its ticket is: the first deploy that brought
it (`deploys`: a production deployment, a deploy workflow's run, or its
commits on main for a project with neither), or, for work in the squad's
folder, when it is done. Then equipe reads the card's metric again **7 and
14 days** later — the same query over the N days after it went live, beside
the N days before — and says:

- **worked** — moved the better way by 5% or more;
- **no change** — less than 5% either way, or fewer than 20 counted;
- **worse** — moved the wrong way by 5% or more.

Each outcome, with its numbers, days and query, goes to the person and the PM.
On **worse**, equipe puts a card to the person to undo it (or try another
way), standing on the follow-up's numbers. A cancelled ticket drops its card.

## Budgets

- **Tokens:** the loop is counted, by equipe's estimate, the run's message
  (four characters a token) plus 20,000 for the PM's turn. Over its weekly cap
  (150,000 by default, `weeklyTokens`) in any 7 days, a run doesn't start or
  isn't handed over. Follow-ups and undo cards use no model.
- **Cards:** at most N open (`maxCards`, default 5, at most 10).
- **Queries:** the connection's ad-hoc budget (60 an hour) and pacing.

## `.equipe/growth.json`

Committed in a project, like `.equipe/checks.json`. Every field is optional.

```json
{
  "keyEvents": ["question_sent", "answer_received", "signup"],
  "funnels": [
    "question_sent → answer_received",
    { "name": "activation", "steps": ["signup", "question_sent", "answer_rated"], "windowDays": 3 }
  ],
  "audiences": [{ "name": "Korean", "where": "$browser_language~ko" }],
  "exclude": ["$host=localhost", "email~@example.com"],
  "excludeEquipe": true,
  "health": [
    { "name": "api", "url": "https://api.example.com/health" },
    { "name": "errors", "hogql": "SELECT count() FROM events WHERE event = '$exception' AND timestamp > now() - INTERVAL 7 DAY", "better": "down" }
  ],
  "allowedFields": [],
  "connection": "m1",
  "maxCards": 5,
  "weeklyTokens": 150000
}
```

| field | what |
|---|---|
| `keyEvents` | events counted each run |
| `funnels` | `"a → b → c"`, or `{ name, steps, windowDays }` (default 1 day to convert, at most 5 steps) |
| `audiences` | `{ name, where }`: the first funnel read again for each |
| `exclude` | conditions on event properties; an event matching any is left out |
| `excludeEquipe` | leave out equipe's own traffic (below); `true` unless `false` |
| `health` | `{ name, url }` (an https GET: up or not, its status and time) or `{ name, hogql, better }` (one number, `up` or `down` is better) |
| `allowedFields` | personal fields a card may name (`email`, `phone`); none by default |
| `connection` | the PostHog connection to read, when the squad has several |
| `maxCards`, `weeklyTokens` | the caps above |

Several projects' declarations are read as one.

## Traffic left out

Every query a run sends ends with `AND NOT (…)`: the project's `exclude`
conditions and, unless `"excludeEquipe": false`, equipe's own traffic.

Conditions are `property=value`, `property!=value` or `property~text`
(contains, any case), on event properties; a property an event lacks is the
empty string.

**The convention for equipe's own browsers** (and any automated or test
traffic): an event is left out by default when

- it has `internal = true` — the convention the PostHog adapter's `filter`
  already uses (`internal!=true`);
- it has `equipe_agent = true`;
- its user agent (`$raw_user_agent`) contains `equipe`.

A product marks its own automated traffic the same way: register
`internal: true` (or `equipe_agent: true`) as a super property for test
accounts, CI runs and the browsers members work in — for example when the page
was opened with `?equipe_agent=1`, or the user agent says so. What isn't
marked can't be told apart; add a condition to `exclude` for it.

## For screens

- `growth.list { squad }` → `Growth { cards, last_run, spent, cap }`: every
  card, newest first, with its evidence, answer, ticket, request and outcomes.
  The desktop shows the latest under **Growth** in Progress.
- `growth.run { squad }` → `RunStarted { started, why? }`: the person's own
  clients only.
- Open cards are ordinary decision sets (`QuestionsAsked`), answered with
  `answer_questions`.
