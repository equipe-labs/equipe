---
name: design-references
description: Use before you design, redesign or judge a screen or component — the reference research for each pattern it uses, the squad's sources, the quality bar and its taste checklist, and when to propose a new foundation.
---

# References and the quality bar

Design to the level of the best products, not to the level of the screen you were handed. Before you design or
redesign a screen or component, study how the best products do each thing it shows; hold your work beside them while
you design and whenever it is judged.

## Research before you design

1. List the patterns the screen uses — data table, KPI tile, filter bar, side navigation, chart, empty state, toast,
   sheet, form… — each named as a folder (`kpi-tile`, `filter-bar`).
2. For each, read the project's `design/references/<pattern>/notes.md` first. Reuse it when it holds at least three
   examples that fit this product and none is older than six months; research only what is missing or stale, and add
   to the notes rather than start them again.
3. Gather three to six best-in-class examples of each pattern from real products, at the primary widths of the
   product's kind (`ui-product-kinds`: 1440 and 390 for a landing page, 1280 or 1440 for a web product, 390 for a
   mobile app): from the squad's sources (below) that fit the kind; from a connected MCP server
   that serves UI references; from public pages in your own browser, or shot with `equipe design refs shoot <url>
   --learn "…" --aspect pattern` (paced, as robots.txt allows). A paid library only through its official MCP server
   or API, when the squad has connected one; a site behind a sign-in is never scraped or captured in bulk. Keep each
   picture in the pattern's folder, `<pattern>/<site>-<width>.png`, or cite the path `refs shoot` kept it at.
4. Write the pattern's `notes.md`, one entry per example:

   ```
   ## <product> — <screen>
   - Source: <URL> (<which source>), <YYYY-MM-DD>, `<picture>`
   - Take: density, spacing, hierarchy, interaction — concretely ("36px rows, label above value, one accent")
   - Leave: what doesn't suit this product
   - Fit: how well it fits this product, and why
   ```

   End it with a line or two on what the examples agree on: that is the bar for this pattern.

References are internal study material, kept in the design folder: take patterns, structure and proportion — never
another product's brand, logo, icons, illustrations, pictures or words. Every line of copy is the product's own.

## The sources

The squad's sources are a file: the project's `design/references/sources.md`, else `design/references/sources.md` in
the squad's folder. A line for each source — what it is, what it is good for, the kinds of product it fits, how it is
reached:

`- <name> — <URL> — good for: <patterns> — fits: <landing | web product | mobile app | any> — access: <public | browser | mcp:<server>>`

`fits:` names the product kinds of the `ui-product-kinds` skill (`any` for writing on craft). Classify the work first,
then use a source only for a kind it fits: a gallery of marketing sites is for landing pages, never the reference for a
dense dashboard; a consumer app's onboarding isn't one for an admin table; a website is never the reference for a
mobile app. An older file's `dense tools`, `mobile` and `marketing sites` read as web product, mobile app and
landing. When neither file exists, start the project's from `references/sources.md` next to this file, keep only the lines that fit the product, and tell the person they can add
their own (a library they pay for and have connected, products they admire). Public design systems' docs and writing
on interaction craft count as sources too: they show a pattern's states and the reasons behind its proportions. So do
component registries (`kind: component registry`), for the parts a real page is built from.

## The quality bar

Every judgment of a screen — yours before the critic, the critic's, the second look's, design QA's — puts it beside
the references of its patterns at the same width and size, and answers one question: is it in the same quality tier?
Where it isn't, name the concrete differences: density, type scale, spacing rhythm, how many borders and surfaces,
information said twice, how clear the primary action is, alignment. Give the reviewers the references:
`equipe design critique <pictures…> --reference <picture>…` and `equipe design second-look <flow/screen> --reference
<picture>…`.

The taste checklist, each a finding where it fails: one primary action per view; no card inside a card; at most two
levels of borders or surfaces; no information said twice; two or three type sizes per block; one spacing scale, kept;
tabular figures wherever numbers line up; the content using the width sensibly — no thin column lost in a wide
window, no line of text across all of it.

Some of it is counted for you, the rest is your eye. Design QA's precheck reports, as polish on the changed screens, a
card in a card and boxes framed three deep (`nested-cards`, `border-levels`), more than three type sizes in one block
(`type-sizes`), a label or heading said twice (`duplicate-text`) and two or more buttons that look primary in the first
screenful (`primary-actions`); `equipe design lint --render` notes more than six type sizes on a screen and boxes framed
three deep. The spacing rhythm, tabular figures where numbers line up, how the width is used, and the tier itself are
judged — by you, the critic, the second look and design QA.

Below the bar is not done: revise with the gaps named, or send it back with them.

## Options that are worth a choice

When options go to the person, each grows from a named reference family — "a dense list in the manner of Linear", "a
data dashboard in the manner of Stripe", "a calm document in the manner of Notion" — and they differ in concept: the
layout model, the density, the hierarchy, not a tweak of spacing, colour or radius. Two strong options beat three
similar ones; a set whose options are near-duplicates goes back before the person sees it.

Build each option as a real page, not a hand-drawn SVG or HTML picture: in the project's stack, on its own component
library (or, where it has none, a proven open one for its stack, such as shadcn/ui on Radix for React with Tailwind),
kept under `design/directions/<name>/`, so what the person picks is what ships. A wireframe may stay a sketch. On
such a stack, look for a proven component in a component registry from the sources first (through its official MCP
server or API when the squad has connected one; `equipe design component` pulls a public one with its source),
restyle it to the project's tokens, and note its source and licence in the pattern's notes. Never paste a premium or
paid component the squad hasn't licensed.

## When the baseline is below the bar

A system the person accepted is locked, and that is good: day-to-day tickets build inside it, for consistency. What
isn't worth locking is a baseline that is itself below the bar. A project's foundation — type scale, spacing,
surfaces, colour, density — can be reset once: when it keeps the screens below their references' tier, don't patch
inside it; propose the reset to the person as a decision (`ask_decision`), with the current screen, the same screen on
the proposed foundation and the references side by side, what changes in the tokens and which screens it reaches.
Carry on with the ticket in the current system meanwhile. The choice is theirs; once made, the reset is a design
system ticket of its own, recorded in `design/decisions/`, and the new system is locked again. The brand's `## Locked`
lines are never part of a reset unless the person says so.
