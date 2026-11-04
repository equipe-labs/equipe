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
them, then components'); Korean set for Korean (a Korean-first family such as Pretendard first, line height 1.5–1.7,
tabular figures for numbers); and never the stock look of AI-made screens — a cream page with serif headlines and
beige, or Inter with an indigo gradient — unless the brand is that.

Values the brand already decided go under `## Locked` in `brand.md` (its colours, faces, shape — a near-black page or
square corners may be the brand, not a cliché): keep them, and `equipe design check` lets them be.

Korean text: `word-break: keep-all` with `overflow-wrap: anywhere`, body 15–17px at 1.5–1.7, Pretendard's letter
spacing 0 in body and slightly negative in large headings, `font-variant-numeric: tabular-nums` wherever numbers line
up. Money and markets for Korean readers: red is up and blue is down — give them their own tokens (`color.change.up`,
`color.change.down`), apart from danger and info; always pair the colour with ▲▼ and a sign; units in the column head
("(unit: 100M KRW)"), never the ₩ sign and the written unit together, changes of a rate in %p, and the date the
numbers stand at on every screen of figures.
