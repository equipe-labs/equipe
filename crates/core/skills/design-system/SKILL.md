---
name: design-system
description: Use when you set up a project's design or work on its brand, brief, logo, tokens, components or guidelines — the `design/` folder, the starter brand and the deep brand system, how a system is built — and its registry.
---

# The design folder, the setup and the system

{{#if role:designer}}
equipe's design system screen shows what these `design/` folders hold, so keep their layout, and bring it up to date
in the same hand-in whenever brand or design work lands: `design/tokens.json` in the W3C Design Tokens format (groups,
each token a `$value` with its `$type`; aliases as `{color.blue.600}`) covering colour, typography, spacing, radius,
shadow and motion (`duration`, `cubicBezier`), each colour with its dark value in `$extensions.mode.dark` or in
sibling `light` and `dark` groups; `design/guidelines.md`, the rules in prose; `design/assets/` with the logos
(`logo/`, an `-on-dark` variant where it differs), icons and illustrations as SVG or PNG; `design/components/` with a
PNG or SVG preview you render of each key component; `design/flows/`, the UI map; and `design/qa/`, design QA's
reports. A draft stays in your worktree until the person accepts it.

A project with no design yet gets its setup in two tiers. Know which one your ticket is: its title says "Starter brand"
or "Deep brand system", or its body has a `Design tier:` line.

**Starter brand** — every new product's default, one ticket, about $3–5 of credits at most, and it must still look
impressive. The tools do the work and you take one short look: in a scaffolded project run `starter_brand` (equipe
platform tool; free, no model): it keeps the design direction's palette (or the existing site's), its type pairing and
`tokens.json` (contrast fixed to AA), runs the gated logo pipeline (`npm run logo`, capped at cents) and writes a short
`brand.md`, a one-page `guidelines.md` and `brand.json`, which the Brand page shows as one brand sheet. Elsewhere: `equipe
design init`, the tokens from the direction or `equipe design audit <site>`, `equipe design specimen --png`, and your
`brand-kit` skill's gated logo step. Then look once at the Brand page (or the specimen): put the voice samples in the
product's language, fix anything plainly wrong (a clashing colour, a logo that reads badly at 16 px), and hand in. Nothing
else: no component sheets, no UI map, no options sheet, no second round. The step runs on a small hard budget; when equipe
says it is used up, hand in what you have.

**Deep brand system** — only when the person chose it (the Brand page's card, which shows its cost) or said yes to the
PM's one suggestion: a ticket a step, each under its own hard budget, in order: 1. `Deep brand system: brief` —
`brand.md` in full, with the person; 2. `Deep brand system: logo round` — only if they want a new mark: genuinely
different candidates that passed the logo quality gate, the team picks one and shows it with why, then light and dark, a
favicon and an app icon in `design/assets/logo/` (the person can switch to another passing one on the Brand page);
3. `Deep brand system: tokens, component sheets and guidelines` — layered `tokens.json`, the `components/` previews and
`guidelines.md`; 4. `Deep brand system: UI map` — `flows/flows.json` and its pictures. Design QA then checks what is
built against them (a report under `design/qa/`). Say which step you are on; make each taste call yourself and show it
with why, and the person can change any of them. When equipe says a step's budget is used up, hand in what is done and
say what is left in one line: never start the next step on your own. The last step sets `"tier": "deep"` in `design/brand.json`, so the
Brand page stops offering the deep system. After the setup, keep what exists growing with the
product: it is never done, but it grows with real requests, not as setup work.

An imported site (its repository has `reference/`: the person brought an existing site in) keeps its original design
until the person asks for a change: its faces, sizes, capitals, colours, gradients, corners, spacing and copy, page by
page. It gets the starter tier as a light setup, in one hand-in, and that setup only reads: harvest the brief (`brand.md`, its values
under `## Locked`), the tokens from the site's own CSS exactly as they are, and a one-page `guidelines.md` describing what
the site does today; put its existing logo files in `design/assets/logo/` as they are (redraw a vector only when the only
logo is a poor raster, the same mark, with no options sheet); and a UI map of its main flow only. What the audit reads as
accidents (two button styles, a second gold, odd corners) are notes for the person, never fixes you apply or tokens you
"correct". Component sheets, the rest of the map and new logo options wait until a request designs a new screen or flow,
and then cover only what it touches, in the site's own look. Never open a ticket to apply or re-apply a design system,
restyle buttons or type, or tidy copy on an imported site: restyling it is the person's request to make, and the ticket
that does it quotes that request. equipe checks this by code before every live deploy: pages whose look moves away from
the original (captured at import in `reference/fidelity/`) with no customer request behind the work are refused. The logo step follows your `brand-kit` skill. The tokens are the source of truth and the code's theme is generated
from them (`web-stack`); a design file in a tool such as Penpot mirrors the repository (`design-loop`).

For the brief and the system, first find out which case it is. A company with a brand already (a website, an app, a
deck): harvest before you design — `equipe design audit <its site>` (more pages with `--pages`), then ask the person
for the logo files, brand guidelines or a deck, and read what they attach. Show them the audit sheet
(`design/audit/brand-audit.png`) saying what you read as the brand (kept) and what as accidents (one-offs), and ask
only what it can't answer. Then build the system around it: keep its colours, type and shape (radii, line weights),
and add what a product needs — neutrals, states, dark values, data colours that don't fight the brand. From nothing:
fill `brand.md` with the person first — who it is for, three to five attributes each with what it is not (this, not
that), two or three references they admire and one thing it must not look like — then three genuinely different
directions, each a tokens file (`design/directions/a.tokens.json`, …) drawn with `equipe design specimen --tokens
<file> --png`: the style tiles, side by side in one comparison with your recommendation and why. Either way:
`brand.md` holds the voice — one register everywhere (formal or casual, buttons and errors too), the words it uses and
avoids, how numbers, money and dates are written, and sample lines for a primary button, an empty state, an error, a
success and a destructive confirm; tokens in layers (primitive values, then roles like `color.action.primary` aliasing
them, then components'); every language the product speaks set for its script (a family made for that script first
— Noto Sans JP for Japanese, Pretendard for Korean, IBM Plex Sans Arabic for Arabic — and tabular figures for
numbers); and never the stock look of AI-made screens — a cream page with serif headlines and beige, or Inter with an
indigo gradient — unless the brand is that.

Values the brand already decided go under `## Locked` in `brand.md` (its colours, faces, shape — a near-black page or
square corners may be the brand, not a cliché): keep them, and `equipe design check` lets them be.

Set type, numbers and dates for the product's own languages and market — the person's language and the people the
product is for — never for one country by default. The script decides the type: Chinese, Japanese and Korean want a
family made for them, body 15–17px at 1.5–1.7 and letter spacing 0 in body, and no break inside a word (Korean:
`word-break: keep-all` with `overflow-wrap: anywhere`; Japanese and Chinese break between characters but keep closing
punctuation off a line's start); Arabic, Hebrew and Persian read right to left — `dir="rtl"`, layout in logical
properties (start and end, not left and right), icons that point the way mirrored, numbers and Latin runs still left
to right inside; Thai and Devanagari need room above and below for their marks. Everywhere,
`font-variant-numeric: tabular-nums` wherever numbers line up. Write numbers, money and dates as the market does,
through `Intl.NumberFormat` and `Intl.DateTimeFormat` with the product's locale rather than by hand: its decimal and
grouping marks (1,234.5 · 1.234,5 · 1 234,5), its currency's sign and where it goes, its date order (month first, day
first, or year first) and a 12- or 24-hour clock. Money and markets: the colour of a rise is the market's — green up
and red down in most, red up in much of East Asia (Korea, China, Taiwan, Japan) with blue or green down — so give change
its own tokens (`color.change.up`, `color.change.down`), apart from danger and info; always pair the colour with ▲▼
and a sign; units in the column head ("(in $M)", "(unit: 100M KRW)", "(in € thousands)"), never a currency sign and
the written unit together; changes of a rate in percentage points (pp, or %p where the market writes it so); and the date the numbers stand at on every
screen of figures.
{{/if}}

## The component system: a registry, each component documented and checked

A project with a web UI (React, Vue or Svelte with Tailwind) keeps its components as a registry of its own, built on
shadcn/ui's open source base (MIT) and restyled to its tokens, so every product is built from it and `npx shadcn add`
installs from it. `equipe design system`{{tools_at}} says what a project has — its tokens (a DTCG `tokens.json`, the CSS variables
its theme sets, a Tailwind `@theme`), `components.json`, `registry.json`, the components' folder, a docs page each —
what is missing, and its coverage against the full set ("47/49 components, missing: …"). Read
`references/registry.md` next to this file before you start a registry, add or change a component, or release one:
it has each step of a component's life — spec, built on the shadcn base, its docs page, its matrix with accessibility
and keyboard checks, version and changelog, release — and how each is checked.
{{#if role:designer}}

When a project has no system yet, its setup builds one from the brand in this order: the brief and the logo, then the
tokens (primitives, roles, dark values), then the registry — every component in the full set, restyled to those tokens,
a docs page each — then the products from it. What isn't there yet, you build; nothing assumes another project's
system or a tool beyond these.
{{/if}}
{{#if role:frontend}}

You build the components the designer specifies, on the shadcn base, restyled only with the system's tokens, each with
its docs page; the designer checks them against the spec. A product screen uses only the registry's components and
tokens — the `ui-floor` skill says how that is held.
{{/if}}
