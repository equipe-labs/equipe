# Design QA: precheck, gate, tiers and measurement

When an engineer hands in a code ticket that changes what users see (`changes_screens: true`), equipe puts a design
QA ticket on the board for the squad's designer, and the hand-in waits in review until the designer passes it or sends
it back. Much of what a designer checks there is deterministic — text cut off, overlaps, sideways scroll, contrast, and
whether a screen changed at all — and checking it by looking at dozens of screenshots is the most expensive work a
squad does. So before any designer turn, equipe runs a **precheck** that needs no model, a **gate** decides what the
report settles, the designer works on what is left on a **cheaper model tier**, and every design QA ticket is
**measured** so the change can be judged on numbers.

## The precheck

`equipe design precheck` (run by the daemon on each hand-in, off its loop, with the design QA ticket held off the
board meanwhile):

1. Reads the ticket's worktree: the files changed since it met the main branch (and any not committed), and the UI
   map (`design/flows/flows.json`). The screens it **touches** are those whose `source` lists a changed file (a file, a
   folder ending in `/`, or a pattern with one `*`), or whose map pictures changed. Changed files that can change what
   users see (styles, markup, components, locale files) and that no screen lists are **unmapped** — see
   [What counts as a UI file](#what-counts-as-a-ui-file).
2. Renders the states of the touched screens, **most relevant first**: states of screens whose `source` lists a changed
   file before those only a changed picture touches (the screen with the most changed source files first), and each
   screen's default state before its others. It stops at `maxStates` (default **40**) or when the render time budget
   (`--budget-secs`, 900 s) is spent — no state is started past it — and **names every state it left out** in the
   report. Each state is rendered at **18 conditions**:
   390, 768 and 1280 px × light and dark × normal, text at 200% (`text-200`, WCAG 1.4.4) and the page zoomed to 200%
   (`zoom-200`, half the CSS width: WCAG 1.4.10 reflow). States shot by URL are rendered in the Mac's own Chrome,
   headless (`precheck.mjs`), on a preview of the worktree; states shot by a `command` run it, at normal size, in light,
   dark and the map's extra `modes`.
3. Measures each rendered page: whether it scrolls sideways and what reaches past the window; text cut off at the left
   edge; text its box cuts off, with what it is (a control's label, a heading, a number, its whole text in a `title`
   or `aria-label`, decoration); text drawn over text; and axe-core's WCAG 2.x A/AA rules.
4. Compares each normal picture, pixel by pixel, with the map's picture committed at the base, and cuts crops of what
   changed (base beside new) and of each finding.

Findings are classed by `crates/core/src/design/precheck.rs`:

| Finding | Blocker (S1/S2) when | Otherwise |
|---|---|---|
| `reflow` — the page scrolls sideways | the viewport is at least 320 CSS px | polish (e.g. 390 px zoomed to 195) |
| `offscreen` — text cut off at the left edge | the viewport is at least 320 CSS px | polish |
| `text-clipped` | content is lost: a control's label, a heading, a number, or any text cut hard without an ellipsis — and its whole text isn't in a `title`/`aria-label` | cosmetic: secondary text cut with an ellipsis or a line clamp |
| `text-overlap` / `overlap` | two texts overlap by 30% of the smaller | cosmetic |
| `axe/<rule>` | impact serious or critical and tagged WCAG A/AA | polish |

A finding is **new** — this change's — when the picture it is on (at normal size, the same width and scheme) changed
against the base and, at normal size, it lies in the part that changed. On an unchanged screen it was there before
and never sends work back.

The report goes in `~/.equipe/squads/<squad>/design-qa/T-<qa>/`: `report.json`, `report.md` (a short summary with the
flagged pictures), `shots/`, `base/` and `crops/`. Reports older than 30 days are removed.

### What counts as a UI file

A changed file no screen lists only holds the change back when users could see it. The heuristics read the path and,
where the path can't tell, the file itself, and name no framework:

- **Visual** — styles (`css`, `scss`, `less`…), markup and templates (`html`, `vue`, `svelte`, `astro`, `erb`, `hbs`,
  `jinja`, `cshtml`, `swift`, `kt`, `dart`, `xaml`…), design tokens, Tailwind config, and locale files under
  `locales/`, `i18n/`, `messages/`, `lang/`, `translations/`.
- **Script code** (`ts`, `tsx`, `js`, `jsx`, `mjs`…) is visual only if it can draw something: a `.tsx`/`.jsx` needs JSX
  (`</`, `/>` or a fragment) in it; other scripts need a UI folder (`components/`, `app/`, `pages/`, `views/`, `ui/`,
  `screens/`, `layouts/`, `src/routes/`) nearer the file than a shared-logic folder (`lib/`, `utils/`, `helpers/`,
  `services/`, `types/`, `config/`, `constants/`, `schemas/`, `models/`) and must not be type declarations only
  (imports, `type`, `interface`, re-exports). A file that can't be read (deleted) is held against the change.
- **Never visual** — tests and stories, docs and READMEs, `.d.ts`, tool config (`*.config.*`, `vite.*`, `jest.*`…),
  anything under `api/`, `server/`, `functions/`, `middleware/`, `migrations/`, `db/`, `scripts/`, `workers/`, `jobs/`,
  `e2e/`, `mocks/`, `fixtures/`, and every other language's source (`.go`, `.rb`, `.py`, `.sql`…) that isn't a template.

Visual unmapped files are `unmapped` in the report; the rest are `unmapped_quiet` (listed, not held against the change).
A shared helper that does change how a screen looks is still caught: the screens that use it list it in their `source`,
or their pictures change.

### What the map says

`flows.json` may carry a `precheck` object:

```json
{ "precheck": { "preview": "npm run dev -- --port {port}", "origin": "http://localhost:3000",
                "ready": "/", "modes": ["light-large", "dark-large"], "widths": [390, 768, 1280], "maxStates": 40, "unmappedLarge": 10 },
  "flows": [] }
```

- `preview` — a shell command run at the root of the ticket's worktree that serves the app on `{port}` (also in
  `$PORT`). The map's URLs written against `origin` are opened on it. Without a preview (or `--url`), states shot by URL
  aren't rendered: the report says so and the designer takes it.
- `maxStates` — the ceiling on rendered states (default 40). The time budget, not this, is what normally stops a big
  change: a state takes about half a minute at its 6 to 18 pictures, so 900 s covers some 30 states, and what doesn't
  fit is left out whole and named. Leave it out unless you want a lower ceiling; an old map's `12` still holds and is
  the reason big changes show many left-out states.
- `unmappedLarge` — how many unmapped UI files make that change large and send the designer to the top tier (default 10).
- `modes` — extra modes a state's `command` is run in, beside `light` and `dark`. A command is also given the
  preview's origin as `{origin}` / `EQUIPE_SHOT_ORIGIN`.

Run it by hand on any worktree: `equipe design precheck --worktree . --base main --out /tmp/precheck`.

## The gate

`crate::design_qa::gate` reads the report:

- **Send back** — one or more new blockers: back to the engineer with the findings and the report, as a designer's
  send-back would (it counts as a correctness send-back for model escalation).
- **Designer** with judgment — what the report leaves unseen or too big to settle without one; the reason is kept
  (`judgment`) and decides the tier:

  | Reason | When | Tier (economy) |
  |---|---|---|
  | `precheck_failed` | no report, or nothing rendered because setup is broken (no map, no preview) | top |
  | `not_rendered` | nothing rendered, or half or more of the pictures failed | top |
  | `partial` | a few pictures failed; the rest rendered | cheap |
  | `capped` | touched states were left out over `maxStates` or the time budget; the rest rendered | cheap |
  | `unmapped` | a few unmapped **UI** files (see above); the rest rendered | cheap |
  | `unmapped_large` | `unmappedLarge` (10) or more unmapped UI files | top |
  | `broad` | 6 or more screens changed (screens, so a raised state cap doesn't make one screen look broad) | top |

  When several apply, the strongest decides and all are said. A redesign or a second round is top whatever the reason.
  The designer on the cheap tier gets the report, which names the states it left out; those are the ones it renders.
- **Auto-pass** — every touched state rendered, none left out, nothing unmapped (UI), no picture changed against the
  base, nothing new found: passed without a designer turn. The ticket's note says `Design QA by equipe (T-…): passed.`
  with the headline, and the person and the PM are told it passed by the precheck. **A state left out never passes**,
  whatever it was left out for.
- **Designer** — otherwise: the changed states and what the report flags.

A failed precheck says why: the gate's reason and the report's Problems list carry the errors, a preview that
doesn't start carries the first lines of its log, a failed picture's cause is counted in the report ("Why pictures
failed"), and a precheck that stopped or wrote no report puts the first lines of `precheck.log` in the ticket's event.

Each squad sets whether the gate acts:

| Mode | What happens |
|---|---|
| `off` | No precheck. Design QA as before. |
| `shadow` (default) | The precheck runs; the designer gets the report; what the gate would have decided is kept beside the designer's verdict, never acted on. |
| `on` | The gate passes and sends back by itself; the designer gets the rest with the report. |

```sh
equipe design qa-config                      # what the squad has now
equipe design qa-config --gate on            # off | shadow | on
equipe design qa-config --tiers economy      # economy | quality | <cheap>/<top>
```

With a report, the designer's brief (`qa_prompt`, and the `design-qa` skill) starts from its summary and flagged
pictures, doesn't re-shoot or re-measure, and spends its look on the spec, the states and what truly costs the person
something; it renders only what the precheck couldn't. The precheck's pictures count as the designer's own for the
pass's proof.

## Model tiers

Design QA tickets run on the squad's tier policy (`crate::design_qa::tier_for`), through the same tier machinery as
every ticket (`crate::tiers`):

- `economy` (default): **standard** (Claude: Sonnet · medium; Codex: Sol · low) — **deep** (Opus · high / Sol · high)
  for a redesign (the checked ticket says redesign, restyle, …), a second round or more, or a precheck whose judgment
  reason needs the top tier (table above; no report at all included, so with the gate `off` design QA stays on the
  top tier). `capped`, `partial` and `unmapped` stay on the cheap tier: the designer is handed what wasn't seen.
- `quality`: deep always, as before.
- `<cheap>/<top>`, e.g. `strong/max`.

Precedence, strongest first: the tier the person set on the ticket; the squad's model choice `off` (members' own
models); a model the person set for the member (`/model`, a move they made, or the squad's default for the role) —
then the member's own model is the floor and the cheaper tier is only recorded beside it (`shadow` in
`model_choices`); otherwise the policy's tier, below the member's own model included.

## Measurement

Every design QA ticket is kept in the squad journal's `design_qa_runs` table: gate mode, precheck state and time,
the gate's decision (would-be in shadow), why it needs judgment (`judgment`), whether it acted, the report, the commits checked,
the round, and the verdict with who gave it. Tokens, tool calls, time and model come from the `turns` table.

```sh
equipe design qa-stats                       # the last 30 days, the squad of this folder
equipe design qa-stats --squad acme --since 7d
equipe design qa-stats --since 2026-11-01 --json
```

It prints the count, verdicts and pass rate; tokens in all and per ticket (cache read and write, input, output); turns,
tool calls and time; rounds; the same by model tier and by gate mode (`before` for tickets from before the precheck,
found by their record); and a count, with tokens per ticket, **per judgment reason** (`capped`, `unmapped`, `not_rendered`…, `none` for a
designer's plain look, `auto_pass`, `send_back`) so the distribution is visible directly; and the gate's agreement with designers — would pass and passed, would pass but sent back
(the risky miss), would send back and sent back, would send back but passed — plus what the gate did by itself. It
opens the journal read-only, so it is safe while the daemon runs.

## Replaying past design QA

`equipe design qa-replay` re-runs past design QA tickets through the new pipeline, read only, to measure it before
switching the gate on:

```sh
# Free: the precheck and its gate only, no model. Start here.
equipe design qa-replay --squad acme --last 30 --dry-run
equipe design qa-replay --squad acme --tickets 2077,2065 --dry-run --json

# With a cheaper-tier designer through Claude Code (uses the plan).
equipe design qa-replay --squad acme --last 10 --designer-model sonnet --designer-effort medium
```

For each design QA ticket with a verdict it finds the commit that was checked (kept since this change; for older ones
the latest `equipe: T-<n> …` commit made by the time of the design QA) and where it met the main branch then, checks
it out **detached in a temporary worktree** under the replay folder (dependencies cloned copy-on-write as live
worktrees get them), runs the precheck and the gate there, and removes the worktree. Live tickets, branches and
worktrees are never touched; the journal is opened read-only. Without `--dry-run` a designer on the given model reads
the report, its pictures and the diff (read-only tools: Read, Glob, Grep, `git diff/log/show`) and gives a verdict; its
tokens are compared with the recorded ones. Everything is kept in `~/.equipe/design-qa-replay/<when>/replay.json`
(or `--out`).

## Limitations

- Change detection compares fresh renders with the map's committed pictures; a map whose pictures were shot with other
  data, fonts or a moving clock reads as changed, which only ever sends the ticket to a designer, never past one.
- A finding on a changed screen at 200% text or zoom can't be placed against the change's region (the layouts differ),
  so it counts as new whenever its screen changed.
- States shot by a `command` get pictures only: no DOM checks, and no 200% conditions unless the map's `modes` give
  them.
- Redesign detection reads English words in the checked ticket.
- The UI-file heuristics read paths and file contents, not import graphs: a shared helper nobody's `source` lists, with
  no JSX, whose change moves a layout is only seen if a touched screen's pictures change.
- Past the time budget a state is left out whole (and so never passes); a repo whose preview is slow gets more
  `capped` than a fast one. States are rendered one batch per kind (by URL, then by command), so the budget stops the
  command states only.
