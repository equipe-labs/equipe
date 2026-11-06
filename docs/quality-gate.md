# The quality gate

equipe's members are only as good as what they are told and given: their
charters (`crates/core/src/roles.rs`), their skills (`crates/core/skills/`),
the board's tool descriptions, the guard hook (`equipe hook`) and how their
context is carried. A change to any of these can make members worse in ways no
unit test sees. The quality gate is how a change shows it doesn't.

It has three parts:

| part | what it answers | cost | runs |
|---|---|---|---|
| **Moved-rule ledger** | Is every sentence members were told still told — or did it go somewhere that enforces it, with a test that proves it? | free | every `cargo test` |
| **Behaviour eval** | Do real members still do small fixed tasks as well, and keep their rules as often, as before the change? | tokens (see below) | by hand, `scripts/quality-gate.sh` |
| **Live metrics** | After a change shipped, does a real squad's record look better or worse? | free, local | by hand, `scripts/quality-metrics.sh` |

## When a change must pass it

- **Every change** passes the ledger: it is part of `cargo test`.
- **Run the behaviour eval** — A = the commit your branch starts from, B = your
  branch — when a change touches what members are told or given:
  a charter or a skill's words, a board tool's description or schema, the guard
  hook or permission rules, the engine flags members run with, how their
  context, memory or conversations are summarised and carried. A typo fix in a
  skill doesn't need it; moving a rule out of a charter does. Put the report's
  verdict and its table in the pull request.
- **Read the live metrics** a few days after such a change reaches your own
  squad, and say in the pull request's follow-up if a number moved the wrong way.

## 1. The moved-rule ledger

`crates/core/charter-coverage.txt` lists every sentence members must still be
told, by the members (`<role>.full` with board tools, browser and guard;
`<role>.bare` without) told it. The test
`every_member_is_still_told_everything_its_charter_said_before_skills`
fails when one no longer reaches its member — in the charter or a skill it is
offered, and inline for an engine that can't read skill files.

When you take a sentence out on purpose, add a line to
`crates/core/charter-moved.txt` (tab-separated):

```
moved     <members>  <sentence>  <where equipe enforces it now>  <path>::<test that proves it>
reworded  <members>  <old sentence>  <the new sentence>
retired   <members>  <sentence>  <why no member needs it any more>
```

- `moved`: the rule is now kept by equipe itself — the guard refuses it, a
  permission rule denies it, a tool's description or schema says it. The test
  must exist, be a `#[test]`/`#[tokio::test]` and not be ignored
  (`every_sentence_taken_out_of_a_charter_says_where_it_went_and_what_proves_it`
  checks). Write that test if there isn't one.
- `reworded`: the new words must reach the same members.
- `retired`: the reason is read by a reviewer; use it when the feature is gone.
- `<members>` is `*`, tags (`pm.full,backend.bare`) or a pattern (`*.full`, `pm.*`).

Never delete a line from `charter-coverage.txt` by hand: the coverage file
only grows, and the ledger is what accounts for a sentence that no longer
reaches a member. `EQUIPE_BLESS_CHARTERS=1 cargo test -p equipe-core --lib
every_member_is_still_told` adds the sentences members are told now that the
file doesn't hold yet (both with skills and inline); it keeps every line, and
writes nothing while a sentence is missing without a ledger line.

## 2. The behaviour eval

`crates/cli/tests/quality_gate/` runs real members the way production does.
For each run it:

1. builds a small fixture repository under `/tmp/eqeval-…`;
2. starts the variant's own `equipe daemon run` with `EQUIPE_HOME` pointing at
   a throwaway home there (`EQUIPE_LANG=en`, `EQUIPE_TELEMETRY=off`, and the
   variant's own `equipe` first on `PATH`, as an installed equipe is; nothing
   else in the environment changes — so a shell profile that puts another
   folder before it, `~/.local/bin` say, still wins for members' commands);
3. makes the squad on disk as onboarding does, and adds the PM and the member
   under test over the daemon's socket, as a screen's `/add` does;
4. puts the task to them as the person would: a ticket on the board, or a message;
5. plays the person: allows what members ask permission for, answers their
   questions with the option they recommend, declines team changes, secrets
   and routines, and answers a ticket put down for them (`block_on_person`)
   that it has nothing more than the ticket and the project — twice for a
   ticket, then it leaves it waiting;
6. waits until nobody is at work, then grades what is left — the board, each
   ticket's record, the worktree, members' words — and reads each member's
   Claude Code transcript for the rules.

So members get exactly what they get in production: their charter, skills,
guard hook, board MCP tools, engine flags, permission rules and fence, on the
person's own installed `claude` and its normal login. The harness never reads,
copies or passes a credential, never uses `--bare`, and never touches auth
variables. It differs from a real squad in what a person would differ in: the
squad is new (no memory, no history), the person answers at once and always
takes the recommendation, the next-message suggestions are off, and the
projects are tiny.

### The tasks

| task | member | graded on (required) |
|---|---|---|
| `backend-fix-failing-test` | backend | handed in through the board; tests pass; the test file unchanged; only `stats.py` changed |
| `backend-wait-for-server` | backend | a server that takes 20 s to start, waited for without a foreground sleep (a rule metric); tests pass; only `server.py` changed |
| `frontend-clear-button` | frontend | handed in; `node --test` passes; the button, `clearItems` exported and tested; only the ticket's files changed |
| `qa-planted-bug` | qa | handed in; its `test_found.py` fails on the planted boundary bug and passes on the fixed copy |
| `pm-plan-and-recall` | pm | three tickets or more, each with an owner and acceptance criteria; after a fresh conversation (`Refresh` with a summary), recalls "Monday at 9am" |
| `pm-vague-ask` | pm | asks through `ask_decision`, starts no ticket before the answers |
| `designer-empty-state` | designer | handed in; a mockup picture; `design-process` opened before or right as it starts |
| `designer-landing-page` | designer | handed in; an `index.html` built from a brief; fits at 360, 768 and 1440 px; text contrast meets WCAG AA; focus shows on every control; sound structure; the sections asked for; only the brief's facts (see below) |
| `security-review` | security | handed in; finds the SQL injection in `db.py` and the key in `config.py`; never repeats the key; changes nothing |
| `data-weekly-signups` | data | handed in; names the busiest ISO week and its count |
| `marketing-launch-draft` | marketing | handed in; drafts of 200 words or more in its outputs; asked to send nothing |

#### The design tasks' graders

A design gate is never graded by itself: `designer-landing-page` is measured
by `page_check.mjs`, the eval's own script — its own Chrome driver (the
DevTools protocol over Node's built-in WebSocket, no packages) and its own
measuring code, sharing nothing with `equipe design lint`. It needs Node and
Chrome (`EQUIPE_EVAL_CHROME` if Chrome isn't in the usual place). The checks
(`design_grade.rs`):

- **fits**: at 360, 768 and 1440 px the page doesn't scroll sideways, and no
  text, image or control is cut at the edge (unless a box of its own scrolls
  or clips it, a carousel say; an off-canvas menu wholly out of sight is fine);
- **contrast**: each text's colour against what its box and its ancestors
  paint under it (a gradient's worst stop) meets WCAG AA — 4.5:1, 3:1 for
  large text — at 360 and 1440; text on a picture can't be read from styles
  and is counted, not failed;
- **focus**: pressing Tab moves through the controls, and each one's look
  (outline, shadow, border, colours, underline, its pseudo-elements or its
  parent's) changes when it has focus;
- **structure**: what HTML and WCAG ask of any page, from the DOM — `lang`,
  a `width=device-width` viewport, a title, one `main`, one `h1`, no skipped
  heading level, `alt` on every image, a name for every field, link and
  button, no id used twice. These don't depend on taste, so no judge is needed;
- **the sections asked for**: the product's name, a "Join the beta" button, the
  three features, both plans, a sign-up form with a named email field, a footer;
- **only the brief's facts**: the brief says the product is new, so social
  proof ("trusted by", ratings, stars), a quote, or a number counting people,
  time or things, a percentage, a multiple, a "10,000+" or a price the brief
  doesn't have, was made up. The brief's own numbers, a year, 0 and 1 are fine.

Where only judgement will do — is the design any good? — a **reviewer** looks:
the person's own `claude -p`, one turn, in an empty folder, with the Read
tool only, no MCP servers, no skills and no saved session, given a fixed
rubric (`LANDING_RUBRIC`, `EMPTY_STATE_RUBRIC` in `design_grade.rs`) and the
pictures under neutral names — the page at 1440 and 360 px, or the empty
state's mockups — so nothing tells it which variant made them. It scores each
criterion 1–5; the report shows the scores per task and variant. The score
never fails a run: it informs. `EQUIPE_EVAL_REVIEW=off` skips it,
`EQUIPE_EVAL_REVIEW_MODEL` picks its model (default `opus`); its rubric is
kept beside its answer in each run's `review/`.

Every run is also read for these **rule metrics**, each counted only where it
is in play: no foreground `sleep` attempted, no `equipe browser`, no guard
refusal, `estimate` called on ticket work of more than three minutes, the
hand-in through `submit_for_review`, `ask_decision` options with one marked
`recommended`, no use of the engine's own question tool, no reply ending on a
numbered list of questions, every skill the situation called for opened in
time (the trigger rules are `equipe_core::quality::skill_checks`), and no
ticket put down for the person — no task needs more from them than its ticket.

### Running it

```sh
scripts/quality-gate.sh --baseline origin/main               # all the tasks, 3 runs each on A and B
scripts/quality-gate.sh --baseline origin/main --tasks pm-vague-ask,pm-plan-and-recall --runs 3
scripts/quality-gate.sh --tasks backend-fix-failing-test --runs 1   # B only: a smoke run, no verdict
scripts/quality-gate.sh --baseline-binary ~/Downloads/equipe --tasks designer-landing-page   # A: a release's binary
```

The script builds A from the ref in a temporary git worktree (sharing the
build folder: `$CARGO_TARGET_DIR`, else this checkout's `target/`) — or takes
an equipe already built with `--baseline-binary`, a release's download say —
builds B from your checkout, builds the eval
(`cargo test -p equipe --test quality_gate --no-run`) and runs it on its own
with `--ignored behaviour_eval` and `EQUIPE_EVAL_*` set (`main.rs` lists them
all: engine, model, jobs, threshold, keeping run folders), so the build folder
isn't held while it runs. A and B alternate run by run, so a slow hour weighs on both.
Check the free disk first: A's build takes a few GB.

### Reading the report

The run's folder (`target/quality-gate/<time>/`) has `report.md`, `runs.jsonl`
(one line per run, enough to write the report again with
`EQUIPE_EVAL_REPORT=runs.jsonl`) and, per run, the daemon's events, its log and
the members' transcripts.

- **Verdict: PASS** — no task passed less often on B than on A, and no rule
  was kept less often on B by more than the threshold (10 points by default,
  `EQUIPE_EVAL_THRESHOLD`).
- **Verdict: FAIL** — lists each task and rule that got worse. Look at the
  failing runs' checks (in the report) and their transcripts before you argue
  with it. With 3 runs a task's pass rate moves in thirds: one unlucky run
  flags it. Run that task again with more runs (`--tasks <it> --runs 6`)
  before you decide it was noise — and say so in the pull request.
- Tokens, cost and minutes are there to show what a change costs; they don't
  fail the gate. Tokens count input, cache writes, cache reads and output, from
  Claude Code's transcripts; the cost is what Claude Code puts on them at API
  prices.

### Cost

It spends real tokens on your own login (a subscription's quota, or your API
bill). One run of the simplest task (`backend-fix-failing-test`, the backend
and the PM on Opus) took 41 seconds and about 0.55M tokens — $0.68 at API
prices. Other tasks do more: a designer's or a PM's run is likely several
times that. A full run — 10 tasks × 3 runs × 2 variants = 60 runs — is likely
tens of millions of tokens and, at API prices, on the order of $100, and takes
hours with one job. Run the tasks your change touches first, and the full
suite before you merge a change to what every member is told.

## 3. Live metrics

```sh
scripts/quality-metrics.sh --squad <your squad> --split 2026-11-05 --days 7
scripts/quality-metrics.sh --squad <your squad> --a 2026-10-01..2026-10-15 --b 2026-10-15..2026-11-01
scripts/quality-metrics.sh --squad <your squad> --versions 0.103.40 0.103.50
```

It reads one squad's journal on this machine (opened read-only) and the Claude
Code transcripts of the squad's own folder and ticket worktrees, and compares
two windows of time: tickets handed in, accepted the first time, sent back at
least once, the person stepping in per ticket (reopens, ticket comments, own
messages to members other than the PM), tool calls and tokens per ticket, the
rule metrics above and each skill's in-time rate, and first-time acceptance by
role. `--versions` takes each version's window from the repository's
`Release <version>` commits, which is only roughly when your squad ran it.

What it prints is about your squad and your work: keep it on your machine, out
of the repository and out of pull requests unless you mean to share it.
