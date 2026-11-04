---
name: design-system
description: Use when you set up a project's design or work on its brand, brief, logo, tokens, components or guidelines — the `design/` folder's layout, the five setup steps and how a system is built.
---

# The design folder, the setup and the system

equipe's design system screen shows what these `design/` folders hold, so keep their layout, and bring it up to date
in the same hand-in whenever brand or design work lands: `design/tokens.json` in the W3C Design Tokens format (groups,
each token a `$value` with its `$type`; aliases as `{color.blue.600}`) covering colour, typography, spacing, radius,
shadow and motion (`duration`, `cubicBezier`), each colour with its dark value in `$extensions.mode.dark` or in
sibling `light` and `dark` groups; `design/guidelines.md`, the rules in prose; `design/assets/` with the logos
(`logo/`, an `-on-dark` variant where it differs), icons and illustrations as SVG or PNG; `design/components/` with a
PNG or SVG preview you render of each key component; `design/flows/`, the UI map; and `design/qa/`, design QA's
reports. A draft stays in your worktree until the person accepts it.

On a project with no design yet, your first work is its setup, five steps in order, each handed in with its
deliverable in `design/`: 1. the brief — `brand.md`, with the person; 2. the logo — three genuinely different options
compared on one sheet, the person picks, then the chosen one in light and dark, a favicon and an app icon in
`design/assets/logo/`; 3. the design system — `tokens.json`, the `components/` previews and `guidelines.md`; 4. the UI
map — `flows/flows.json` and its pictures; 5. design QA — what is built checked against them, a report under
`design/qa/`. Say which step you are on; the taste calls in each are the person's. After the setup, keep all five
growing with the product: they are never done.

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
the written unit together; changes of a rate in percentage points (%p); and the date the numbers stand at on every
screen of figures.
