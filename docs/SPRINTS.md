# Sprints

Once a squad's product is live, equipe runs it the way a startup team runs
its product: in sprints. Each sprint ships a few ranked items, measures them
against rules fixed before they shipped, and ends with a short retro and the
next sprint's proposal. The person approves with one tap; the loop runs
itself.

```
            ┌─────────── PM: plan_sprint (retro + next sprint) ◀──────────┐
            ▼                                                              │
 proposed ──tap──▶ active ──▶ tickets ship ──▶ live ──▶ judged by rule     │
  (card)            │                                    (worked / didn't  │
                    │                                     / inconclusive)  │
                    └──── at its slot: review (facts, results, numbers) ───┘
```

Code: `crates/core/src/sprint.rs` (the rules, tested alone),
`crates/core/src/sprint/data.rs` (where product numbers come from),
`crates/core/src/runtime/sprint.rs` (scheduling, gathering, cards),
`crates/protocol/src/sprint.rs` (what screens read).

## When it runs

The loop is **automatic**: it runs for a squad that has a PM and a product
live — a deploy seen (`.equipe/deploy.json`, a deploy workflow, commits on
main; see [REQUESTS.md](REQUESTS.md)) or a PostHog connection. Before that,
nothing changes. The person can turn it on early, off, or back to automatic
(`sprint.set`, `equipe sprint set`).

Each minute equipe looks at every squad's loop:

| state | what happens |
|---|---|
| no sprint, none proposed | the PM is handed a **planning** (at most once every 6 days, or when the person asks) |
| proposed | waits on the person's card; small items may start on their own (below) |
| active, its slot reached | a **review** is gathered off the loop and handed to the PM |
| in review for a day without a plan | closed on its facts alone; a planning follows |

A sprint runs **7 days** (or 14) and is reviewed at the squad's **slot**:
the weekly retro routine's (Friday 17:00 by default), or one the person sets.
A sprint started mid-week runs to the first slot after half its length, so
one started on a Thursday is reviewed the next Friday, not the next day.

While the loop runs, the sprint review **is** the weekly retro and the weekly
growth run: the retro routine is skipped and the growth run doesn't start, so
the PM gets one turn per sprint, not three. Growth cards already open carry on
as before ([GROWTH.md](GROWTH.md)).

## Planning: `plan_sprint`

One PM tool, one call — at a review it also writes the retro:

| field | what |
|---|---|
| `retro` | at a review: `shipped`, `learned`, `wrong`, `next` — at most 3 short lines each, for a non-developer |
| `north_star` | the one number that says the product works for its people (`name`, `metric`); needed once |
| `goal`, `done_when` | what the sprint is for, tied to the north star, and what done looks like |
| `market_guess` | while the person hasn't said: sold to people, to businesses, or not sold |
| `items` | at most 5: `kind` (iterate, fix, kill, new, marketing, measure), `title`, `change`, `why`, `evidence` (`E1`, `H-2`), `expected`, `impact`, `size`, `risk`, `owner`, `project`, `outside`, `hypothesis` |

Checked in code (`sprint::plan`), all or nothing: every item says what it
stands on (`why`, and any ids it cites must exist), names no one, has an owner
on the team if it names one; no plan while a sprint runs or a proposal waits.

equipe **ranks** the items by expected impact against cost and risk
(`sprint::rank`) and **recommends** the strongest that fit the sprint's budget.
Cost is equipe's estimate by size: S 2M, M 6M, L 15M tokens.

## The person's card

One decision card, the same every decision uses:

1. **Start sprint S-4?** with the goal, done-when, budget and the recommended
   items — *Start with the recommended items* (one tap), *Start with the items
   I tick below*, *Not now*.
2. The items to tick, the recommended marked; an item that publishes, posts,
   emails or spends says so.
3. While unanswered once: **Will people pay for this product?** — sold to
   people (B2C), to businesses (B2B), or not sold — the PM's guess marked.

Ticked items win; *Not now* (or closing the card) starts nothing and leaves
the items as "later" for the next planning. Started items become tickets on
the board (`S-4 P-7: …`), created by the person, each serving a request of its
own like any the person makes.

### Starting on its own

Autonomy kind **`sprint`** (`Small sprint items` in settings; ask by default):

- **Ask** — nothing starts without the tap.
- **Decide and tell** — after the squad's window the proposal's small items start.
- **Just do** — they start when the sprint is proposed.

Only recommended items of size S and low risk start this way — never
go-to-market work, never anything marked `outside`, and never once the sprint
has used its budget.

## Hypotheses: measured after ship

An item whose effect can be measured carries a **hypothesis**, with its
**decision rule fixed before it ships**:

```json
{
  "belief": "A shorter signup form gets more people to sign up",
  "metric": "visit > signup",
  "rule": { "min_change_pct": 10, "window_days": 7, "min_sample": 200, "better": "up" },
  "split": { "flag": "short-signup", "control": "control" },
  "events": ["signup_started"]
}
```

- **Metric**: `event:<name>` (a count), `<a> > <b>` (the share of people who
  did a then b within a day), an evidence's metric from the review (its days
  kept), or an object with `days` and a `filter` (`prop=value`, `prop!=value`,
  `prop~text`).
- **Rule**: the change it must make the better way (in % of before), the days
  after going live it is read over, and the fewest counted (events, or people
  for a share) on each side.
- **Split** (optional): an A/B test by a feature flag the product reads; a
  share metric only. PostHog's `$feature/<flag>` property assigns each person
  their variant.

The item's ticket carries the hypothesis, the events to send and the rule.
When the squad has a **data analyst**, equipe also gives them a ticket to check
the new hypotheses' metrics and rules before they ship; they (or the PM) refine
one with **`define_experiment`** — any ticket can get one this way. Once the
ticket is **live** (the same rule as growth cards), the rule is **locked**.

- **The day after** going live equipe looks whether the metric's events
  arrive; none arriving is told to the PM and the ticket's member.
- **After the window** equipe reads the numbers and judges (`sprint::judge`):
  - **worked** — moved by the rule's change the better way, beyond chance
    (|z| ≥ 1.96: a two-proportion test for a share, a Poisson test for a count);
  - **didn't** — short of the rule's change, or the wrong way beyond chance (`worse`);
  - **inconclusive** — fewer counted than the rule asks, a move by the rule's
    change that could still be chance, or numbers that couldn't be read.
  A split compares the strongest variant with control, each arm needing the
  rule's sample. Without flags where the numbers are read, it falls back to
  before beside after and says so.

The result keeps the numbers, the days and the query that read them.

## The review

At the sprint's slot (or when the person ends it early) equipe gathers, off
the loop: hypotheses due (judged now, so the review tells their verdicts), the
product's numbers the growth loop would read (key events, funnels, kept
snapshots, health, signals, checks), and the north star over the sprint's days
beside the days before. Then one compact message to the PM:

- the goal and done-when; tickets done of all, shipped, what rubbed, waits on
  the person, tokens used of the budget;
- each ticket, each hypothesis with its verdict and numbers;
- what rubbed: equipe's own flow (sent back, failed, stuck, late turns, turn
  endings) and each member's friction — the team's inputs, without spending
  their turns;
- the north star, the product numbers (`E1`…), hypotheses tried before, items
  the person left for later, go-to-market, what isn't measured.

The PM answers with one `plan_sprint` call: the retro (closing the sprint)
and the next sprint's items. Open tickets carry over into the next sprint.

## Go-to-market

The person's answer on the first card says whether the product is sold. For a
product sold to people or businesses:

- the review and planning ask for go-to-market items — positioning, who it is
  for, channels, the launch, landing copy, a content calendar, posts — and
  acquisition experiments with hypotheses (a metric with a `filter` such as
  `utm_source=newsletter`);
- without a marketer on the team, the PM is told to propose adding one
  (`propose_team_change`);
- the marketer's `marketing-playbook` skill covers the work. **Publishing,
  posting, emailing and spending always wait for the person's card** — the
  autonomy floor (`outside`, `money`) and the marketer's own limits; items
  marked `outside` say so on the card and never start on their own.

For an internal tool, marketing items are refused.

## Budgets and guardrails

- **The sprint's budget**: tokens its tickets' turns may use (25M by default,
  `sprint.set { budget_tokens }`), counted from the journal every 10 minutes.
  At 80% and 100% the PM and the person are told once; past it nothing starts
  on its own. Each ticket's own budget and every spend guard
  ([HEALTH.md](HEALTH.md)) apply as ever.
- **The loop's own turns**: one review per sprint, a planning at most every 6
  days; each counted (message ÷ 4 + 20,000) in the store.
- **The PM's context**: the sprint is a short block (`sprint`) in front of its
  desk turns, said once per conversation and then only as it changes
  (`turn_context`). The rules and the long instructions travel only with the
  review or planning message that needs them.

## For screens (protocol)

Methods (JSON-RPC over the daemon's socket, and the gateway for remote
screens):

| method | params | answer | who |
|---|---|---|---|
| `sprint.get` | `{ squad }` | `SprintView` | any screen (a read) |
| `sprint.end` | `{ squad }` | `Ended { started, why? }` — the sprint under way is reviewed now, or a first one planned | the person's own clients |
| `sprint.set` | `{ squad, on?, auto?, weekday?, hour?, length_days?, budget_tokens? }` | `SprintView` | the person's own clients |

Event: **`sprint_changed { squad }`** whenever the loop changes (a sprint
proposed, started, reviewed or closed; an item started; a hypothesis locked or
judged; settings). Screens read `sprint.get` again.

`SprintView` — the three sections a screen shows:

| field | for |
|---|---|
| `current` (`Sprint`), `current_done`, `next_review` | **This sprint**: goal, done-when, its tickets (and how many are done), its budget and spend, when it is reviewed |
| `last` (`Sprint` with `retro`, `facts`) | **What we learned**: the retro's lines and what equipe counted |
| `proposed` (`Sprint`) | **Next sprint (proposed)**: answered on its decision card (`QuestionsAsked`, the set whose id is its `set`) |
| `items` | the items of those three sprints: rank, recommended, impact, size, risk, cost, status, ticket, hypothesis |
| `experiments` | their hypotheses, and any other still measuring, newest first: rule, status, `result` (`verdict`, `why`, `worse`, numbers, days, query) |
| `north_star`, `market`, `measurable` | the number, whether it is sold, whether hypotheses can be judged |
| `on`, `why_off`, `settings`, `weekday`, `hour`, `length_days`, `budget_tokens` | the loop's switch and its settings in force |

Autonomy: `Kind::Sprint` (`"sprint"`) joins the levels the person sets
(`Policy.sprint`, ask by default).

## Product data providers

`sprint::data::ProductData` is the one interface hypotheses are read through:

```rust
pub trait ProductData: Send + Sync {
    fn source(&self) -> String;                         // "posthog m1"
    fn splits(&self) -> bool;                           // has feature flags
    fn query(&self, metric: &Metric, from: i64, to: i64) -> String;
    fn read(&self, metric, from, to) -> Fut<Result<Reading, String>>;
    fn read_split(&self, split, metric, from, to) -> Fut<Result<Vec<Arm>, String>>;
}
```

Days are whole UTC days. **PostHog** (`data::PostHog`) reads through the
squad's connection with read-only HogQL — the key stays in the daemon, the
growth loop's exclusions (equipe's own traffic, `.equipe/growth.json`'s
`exclude`) apply. Another analytics and flags platform is another
implementation; a host embedding the daemon registers one per squad with
`data::register` (tests do the same with `data::Fake`).
