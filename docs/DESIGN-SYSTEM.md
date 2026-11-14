# The design system

A squad's design system is a folder its designer keeps: the colours, type,
spacing, radius, shadow and motion of what the squad makes, its rules in
prose, and its logos, icons and pictures. equipe shows it on the desktop
app's **Design system** screen (the sidebar, or `/design`), the terminal
summarises it (`/design`), and a phone can read it — always through the
daemon, so a screen on another Mac or the phone never reads the daemon's disk.

## Where it lives

| Whose | Folder |
|---|---|
| A project's own | `<repository>/design/` — its tokens, components, brand and mockups, versioned with its code |
| The squad's | `~/.equipe/squads/<squad>/home/design/` — only the principles every project shares |
| A draft | `design/` in the worktree of a ticket **in review** — shown as "In review · T-n" until the person accepts it |

A project's design assets are made in its repository from the start, never
drafted in the squad's folder and carried over later. The designer's work on
them is a `code` ticket on that repository, and on it the designer changes
only `design/`: `submit_for_review` refuses a designer's code-ticket hand-in
whose worktree changes anything outside it — committed or not, new files
included — and names the files, to undo and propose to an engineer
(`runtime::design_qa`). Applying the system to screens is an engineer's
ticket. The designer's own code ticket skips design QA. A task ticket still
lands the squad's `design/` on acceptance, for the shared principles.

A folder counts as a design system when it holds `tokens.json` or
`guidelines.md`. A project's `design/` without either is just a folder.
The screen shows the squad's first; a switcher appears when projects have
their own. A draft is shown only where it differs from the accepted one.

```
design/
  tokens.json          W3C Design Tokens: colour, type, spacing, radius, shadow, motion
  guidelines.md        the rules, in prose (shown with its contents)
  assets/
    logo/              mark.svg, wordmark.svg, mark-on-dark.svg …
    icons/
    illustrations/
  components/          a rendered preview (PNG or SVG) of each key component:
                       button.png, its variants and states as button/outline.png
                       (or button.hover.png); button.md says what it is for
  checks/components/   each component's matrix (`equipe design matrix`), its sheets beside
```

equipe's own is [`design/`](../design/README.md) at the top of this
repository: its layered sources are in `design/tokens/`, and `npm run build`
there writes `design/tokens.json` (light as `$value`, dark in
`$extensions.mode.dark`) — a generated file, never edited by hand.

### Counts and what waits (equipe's own screens)

How the desktop app shows that something waits on the person, derived from
`design/guidelines.md` — "The person's approval shows the most … an area has
one primary action" (principle 1), "Never say it with colour alone. A status is
colour + symbol + words" (principle 2), "Only two kinds of colour: the brand
blue (just one) and the semantic colours. Everything else is warm neutrals"
(§2.1), orange (`wait`) = "waiting on the person" (§2.2), and brand §4: icons
follow the text colour, "Colour is only for the status icons (`ok/work/wait/err`)":

1. **One filled badge.** The total of what waits on the person (the sidebar's
   Needs you) is the only count drawn as a filled `wait` pill, figures in
   `on_accent`. A screen has at most one.
2. **Its breakdown is ink.** Every other count — a project's share of the
   total, Progress, a team's "N your turn" — is a number in `muted` (words in
   `muted`/`text2`), never in orange. A count of things waiting is told apart
   by the **dot** before it: 6 pt, in the state's colour (`wait` orange,
   `err` red for work that needs a look), read as a graphic at 3:1.
3. **The dot is the per-item mark.** A conversation, a member's face and a
   project header that wait on the person carry the same orange dot; the
   words beside it ("Your turn") stay in ink, a step stronger (`text2`, medium).
4. **One column.** Counts sit centred in one 22 pt box at the row's end (the
   badge, the numbers, a spinner), their dots in one column before it, so
   rows line up whether they say 1 or 10.
5. **Words for the number.** A count's tooltip and accessible name say what
   it counts ("4 need you", "Needs you, 20").

So when every project waits, the sidebar shows one orange shape and a quiet
column of dots, not a column of orange figures: the signal is the total; the
rows say where.

## Done means on a screen

A design system, token or component change is done when it is on a real
screen of at least one platform, checked with before and after screenshots —
or when the designer has proposed the code ticket that applies it, one for
each platform. Colours and sizes are judged on a real screen of the product,
not only on a sheet of swatches. A component's rules (`components/*.md` and
its preview) are the spec the platform ticket turns into a component in that
platform's code; design QA compares the built screens with that component
and the sheet. What the designer couldn't confirm — a licence, a platform
default, a taste check — goes up as a proposal ticket each, in the same
hand-in. The designer's charter says all this (`roles.rs`).

## Copies

Another repository may use a generated file — the CSS, Swift, Kotlin or
Rust of the tokens — as a copy, never edited there. Each copy is listed in
the source's `design/copies.json`:

```json
{
  "source": "https://raw.githubusercontent.com/<owner>/<repo>/main/design/",
  "copies": [
    { "from": "build/swift/EquipeTokens.swift", "repo": "equipe-app", "to": "ios/Shared/EquipeTokens.swift" }
  ]
}
```

and checked two ways:

- **From the source**, after a build: `npm run check:copies` in `design/`
  compares each copy byte for byte with its generated file, finding the
  other repositories next to this one (`../<repo>`) or where given
  (`npm run check:copies -- equipe-app=~/src/equipe-app`). A copy that
  differs, is missing, or a repository not checked out fails it.
- **In each repository that copies**, one CI line per copy, so a hand edit or
  a stale copy fails its build:

  ```sh
  curl -fsSL "$SOURCE/build/swift/EquipeTokens.swift" | diff -u - ios/Shared/EquipeTokens.swift
  ```

  (`$SOURCE` is `source` above; a private source checks it out instead.)

To update a copy: change the tokens, `npm run build`, copy the generated file
again, and commit both — the source first, so the copy's CI compares with it.

## `tokens.json`

The [W3C Design Tokens Community Group format](https://tr.designtokens.org/format/):
nested groups; a token is an object with `$value` and `$type` (or a `$type`
inherited from its group), and may have `$description`.

```json
{
  "color": {
    "$type": "color",
    "bg":     { "$value": "#F6F5F1", "$extensions": { "mode": { "dark": "#0A0A0C" } } },
    "accent": { "$value": "#1D4ED8", "$extensions": { "mode": { "dark": "#8FB0FF" } } },
    "link":   { "$value": "{color.accent}" }
  },
  "font": {
    "sans": { "$type": "fontFamily", "$value": ["Inter", "Pretendard Variable", "system-ui"] }
  },
  "type": {
    "body": { "$type": "typography", "$value": { "fontFamily": "{font.sans}", "fontSize": "15px", "fontWeight": 400, "lineHeight": "25px" } }
  },
  "space":  { "$type": "dimension", "2": { "$value": "8px" }, "4": { "$value": "16px" } },
  "radius": { "$type": "dimension", "md": { "$value": "8px" } },
  "shadow": { "card": { "$type": "shadow", "$value": { "color": "#0000001A", "offsetX": "0px", "offsetY": "1px", "blur": "3px", "spread": "0px" } } },
  "motion": {
    "fast": { "$type": "duration", "$value": "120ms" },
    "out":  { "$type": "cubicBezier", "$value": [0.33, 1, 0.68, 1] }
  }
}
```

- **Aliases**: a value `"{color.accent}"` (whole, or inside a composite such
  as a typography's `fontFamily`) takes that token's value — and its type,
  when the token has none. Chains resolve; a cycle or a name that isn't a
  token is listed as a problem, and the rest still shows.
- **Light and dark**: put a token's dark value in `$extensions.mode.dark`
  (its `$value` is light), or keep two sibling groups named `light` and `dark`
  (`color.light.bg`, `color.dark.bg` — one token, `color.bg`). An alias of a
  token with modes has those modes too; `{color.dark.accent}` names one mode.
- **Types read**: `color` (`#RGB[A]`, `#RRGGBB[AA]`, `rgb()`/`rgba()`, or
  `{ colorSpace: "srgb", components, alpha, hex }`), `dimension` (`px`,
  `rem`, `{ value, unit }`), `fontFamily`, `fontWeight` (a number or DTCG's
  names), `typography`, `shadow` (one layer or a list), `duration`,
  `cubicBezier` (four numbers, or CSS's keywords), `transition`, `number`.
  Any other type is listed as written.
- **Where a token shows**: colours under Colour; type styles, families,
  weights and type sizes under Typography; dimensions named `radius`/
  `corner` under Radius, the rest under Spacing; shadows under Shadow;
  durations, curves and transitions under Motion.
- Limits: `tokens.json` up to 1 MB and 4000 tokens; `guidelines.md` is sent
  up to 256 KB; up to 240 pictures.

## Pictures

Images in `assets/` and `components/` — PNG, JPEG, WebP, GIF or SVG — are
read through the daemon's `read_image` (the same transport a decision's
pictures use), so a screen elsewhere gets them too. They are given only when
they are inside a design folder (links resolved, no `..`), what their name
says they are, and no bigger than the daemon's limits; an SVG that runs a
script or loads another file isn't. A logo named `…-on-dark` or `…-dark` is
shown only on dark, `…-light` only on light, any other on both.

## The screen

- **Overview**: the logos on the system's own light and dark backgrounds, the lead
  colours (brand, accent, background, text), a type sample, and how much is in
  each section. Problems reading `tokens.json` are listed here.
- **Colour**: swatches by group, light and dark side by side, each with its hex
  and WCAG 2.x contrast (AAA 7:1, AA 4.5:1, Large 3:1) against the
  system's background — or, for a surface, its text. A click on a colour
  copies its hex; on a name, `{the.token}`.
- **Typography**: each style set live in its font, size, weight and line
  height, in the person's language and English. A font this Mac doesn't have
  says so and names the one it is shown in.
- **Spacing · Radius · Shadow · Motion**: specimens at real size; a motion token
  plays its duration and curve under the pointer.
- **Components**: every component the system has, always there (with none
  yet, what it will show and a button that asks the designer — or the PM —
  for previews). Each is listed from what is already kept, never a fixed
  list (`crates/core/src/design/gallery.rs`): a project's `registry.json`
  items (`registry:ui`, `registry:component`, `registry:block` — title,
  description, first file, and `meta.docs` as the page to open), its
  components' folder, the previews in `components/` named as the component,
  and its matrix (`checks/components/<name>.json`: what to fix, the variants
  its docs page shows, the contact sheet at each width — found by name beside
  the record, so a record made in a worktree reads the same once merged).
  A card shows the first preview on a checkerboard, the title, where it
  stands — "In review · T-n" or "In the works · T-n" when a ticket's draft
  adds or changes it (its ticket a click away), else Done — "Checked" or
  "To fix: n" from its matrix, its description, its other pictures, its
  variants and states, and its source file. A click shows its pictures big
  (←/→ through them); "Open live" opens its page (an address anywhere, a file
  on the daemon's own Mac only); "Talk" points the designer at it.
- **Assets**: pictures on a checkerboard, big in a lightbox (←/→, Esc), saved
  to Downloads (from any Mac) or shown in Finder (this Mac).
- **Guidelines**: `guidelines.md`, with its `##`/`###` headings as contents.
- **Changes**: the tokens added, removed or changed since the person last
  looked (kept per daemon, squad and system in `~/.equipe/desktop-design-seen.json`).
- With none yet: what a design system is, and a button that asks the
  designer (or the PM, to bring one in) for one through the composer.

## How it travels

- `command` `{ "type": "design", "squad" }` → `event`
  `{ "type": "design", "squad", "systems": [DesignSystem…], "folder" }`
  (`crates/protocol/src/design.rs`): each system with its `project`
  (absent: the squad's), `dir`, `draft` (`{ ticket, title }`), `modes`,
  resolved `tokens` (`{ name, type, value, modes?, description?, alias? }`),
  `problems`, `guidelines`, `assets` (`{ path, name, kind, size }`),
  `components` (`{ name, title, description?, block?, previews: [{ path,
  label?, sheet?, size }], variants, states, page?, source?, check?: { at,
  fails, notes }, work?: { ticket, title, review? } }`, left out when there are
  none) and a `stamp`.
- While any screen follows the squad, the daemon looks at the folders every
  1.5 s — their files' sizes and times only, and a project's `registry.json`,
  `components.json`, components' folder and matrix records — and sends the
  event again when one changed, so the screen follows what the designer saves.
- `read_image` `{ squad, path, max? }` for each asset.
- The designer studio (below) asks `design_studio` `{ squad, ticket }` →
  `Studio` (`crates/protocol/src/studio.rs`) every 2 s while it is open: what
  the ticket's worktree design folder holds — each screen's latest check and
  second look, the edits not checked yet, the directions' log, the critiques,
  the reference board — and every other picture of the work, each with where
  it came from: the phone's recordings (`<squad home>/recordings/T-<n>/`), the
  ticket's hand-in folder (`outputs/T-<n>…/`) and its reports, the pictures
  the member opened or wrote while working on the ticket (read from its
  conversation's lines, `Journal::ticket_lines`) and those beside them, what
  is new in its worktree since the work began, what its hand-in names.
  Read-only, every file and list capped (`crates/core/src/design/studio.rs`);
  its pictures through `read_image`, which gives what a studio answer named
  for the squad for 15 minutes after. The read-only real view may ask it;
  phones are not routed it.

See `docs/remote-protocol.md` for the phone's side.

### The designer studio (desktop)

Watch a ticket's design work as it happens (`crates/desktop/src/views/studio.rs`;
from a ticket's card, the member's lane in Orchestration, or the conversation's
header when the member is on a design ticket). Under the title, an honest status
line — where the member is, weighed as the sidebar weighs it (an OK asked, a
waiting ticket, a question of its own, on this ticket or another: "Waiting for
you on T-60: …"), else handed in, working or idle — and what holds the work up
("Checks failing: Contrast — …"); what it is doing now, its last ten steps in
plain words with the latest lit and how long it has run; the steps of its kind
of work — a design's brief, references, directions, build, checks, hand-in; a
design QA's brief, the change, the screens seen, measured, findings, verdict;
building screens' brief, build, checks, hand-in — each lit from what the member
did, with how much and when; the newest picture of the work (a check's at each
of seven widths, a phone recording, a screenshot it opened, its hand-in folder's
sheets — never "nothing drawn" while one exists), with the recent ones to pick
from; its report's findings; and a timeline of what it left —
references with what to learn from them, directions and the pick, critiques,
each check and second look, each hand-in and why it was refused — any of them
opened big. The member's conversation is docked at the right, as on the design
screen. The rules (`crates/desktop/src/studio.rs`) read only the member's own
lines on the ticket (Claude's and Codex's steps alike) and the folder above —
never guessed. `studio:<ticket>` in the snapshot script; `FAKE_STUDIO=1|waiting|
failing|handed|codex` in the fake daemon.

## The designer's tools — `equipe design`

What a designer (or the person) runs in a worktree; the designer's skills
say when (`crates/core/skills/design-tools/` and the rest).

| Command | What it does |
|---|---|
| `init [dir]` | Lays out a `design/` folder to fill in, only what is missing: `brand.md` (who it is for, attributes *this, not that*, references, voice and tone with sample lines in the product's language), a layered `tokens.json` (palette → roles → components, dark values, a neutral Noto Sans and system stack to replace with the brand's faces) and `guidelines.md`. |
| `audit <url> [--pages n] [--out dir]` | Harvests an existing brand from its site in the Mac's own Chrome (Playwright, `channel: chrome`): colours by the area they cover, their role (background, text, border) and where; the logo's own colours from its file (an SVG's exact values); type families, loaded webfonts and the type scale; radii and shadows; logo, icon and og:image files; its voice (title, description, headlines, buttons, navigation). Writes `brand-audit.{json,html,png}` and a screenshot per page. |
| `specimen [dir] [--tokens file] [--png]` | Draws a system from its tokens alone: two real screens light and dark (KPIs, a table, fields with an error, buttons in their states, an empty state), the palette with dark values and contrast, type in Korean and Latin, spacing, radius, shadow. A direction's tokens file makes its style tile. |
| `render <file…> [--widths 390,1280] [--dark] [--out dir]` | Draws HTML or SVG mockups to PNG at each width, light and dark. |
| `check [dir] [--json]` | What to fix before handing in: tokens the screen can't read, no semantic layer, no component tokens, no dark values, text under AA (a component's text against its own fill), no family for the script the product is written in (read from the language of `brand.md` and `guidelines.md` — Korean, Japanese, Chinese, Arabic, Hebrew, Thai, Devanagari; right to left said where it applies), body line height, no `brand.md` or voice, no logo or dark logo, no component previews, the stock looks of AI-made screens. Exits non-zero on errors. |

| `lint <files\|folder\|url> [--render] [--url …] [--widths …] [--tier floor\|all] [--screen flow/screen] [--json]` | Holds a screen to its design system in code (below). Exits 1 when the floor fails, 2 when habits are left, 0 when clean. `--waive <id> --why "…"` keeps a habit; `--rules` lists every rule. |
| `second-look <flow/screen> [--first] [--criteria …] [--reference <picture>…]` | A fresh, read-only reviewer judges the checked pictures, given only them, the brief and criteria, the system's guidelines and tokens, and the references of the screen's patterns when given — then it also answers whether the screen is in their quality tier (`tier`: `same`, `below`, `no references`; `below` is never a pass); its verdict is kept on the screen's record. `--prompt` / `--answer <file>` for a fresh subagent when no engine can run. |
| `direction propose\|choose\|list` | The directions proposed and chosen (`design/directions/log.json`) by layout, type, palette, density and motion; a proposal that repeats a recent chosen one in every dimension the brand leaves free, or differs from its fellow options only in colour, is refused. |
| `kept record\|check <files\|url> --screen …` | A screen's links, fields, actions and headings, recorded before a redesign (`design/kept/`) and checked after; a removal only where the brief or a decision says `Remove <kind>: <item>`. |
| `refs sources\|search\|shoot\|add\|board\|list` | A brief's reference board, below. |
| `palette <Name=text-bg-primary-secondary-accent[/dark]>… [--fonts H/B] [--page file\|address]` | One to four palettes on the page being designed (`--page`, else `./index.html` when there is one), light and dark (dark derived when not given) — its own colour variables set to each palette's roles by their names, or its page, links and buttons coloured directly where it has none — and on a page of our own where there is no page yet; the WCAG contrast of each pair text is set in (text on background and on the secondary surface, the primary as a link, a button's label on it, the accent as a graphic at 3:1); each palette's tokens in this document's format in `design/palettes/<name>.tokens.json`, and one comparison sheet `palettes.{html,png}`. |
| `type <Heading/Body>… [--locales en,ko,…\|all]` | One to four Google Fonts pairs set as a specimen — headline at 56, 36 and 24, lede, body, UI labels, figures — and each other language's headline and body; what each family lacks of each language (from the `unicode-range`s Google Fonts serves), its licence (from its folder in Google Fonts' repository) and what the specimen loaded of it. `design/type/specimen.{html,png}`. |
| `component <page \| registry item \| npx shadcn add line>` | A public component's source into `design/vendor/<name>/` with a `SOURCE.md`: where it came from, when, its licence and dependencies, and every colour, palette class, family and remote picture to restyle to the tokens. A registry its site closes to tools (robots.txt) or that wants a key is not fetched: its page and licence are recorded, and that is said. |

### Reference research and the quality bar — every screen

A designer's screens are held to the level of the best products, not to the
level of the screen it was handed (the `design-references` skill,
`crates/core/skills/design-references/`; the role text says it in one line).

- **Research before designing.** For a screen or component, the designer lists
  the patterns it uses (data table, KPI tile, filter bar, side navigation,
  chart, empty state, toast, sheet…) and, for each, keeps three to six
  best-in-class examples from real products in the project's
  `design/references/<pattern>/` — their pictures, and a `notes.md` with each
  one's source and date, what to take (density, spacing, hierarchy,
  interaction), what to leave, and its fit for this product. Notes are reused
  on later tickets and refreshed when stale (older than six months) or thin.
  References are internal study material: patterns, structure and proportion,
  never another product's brand, assets or words. A paid library only through
  its official MCP server or API when the squad has connected one; a site
  behind a sign-in is never scraped.
- **Sources are per project and squad**: `design/references/sources.md` in the
  project, else in the squad's shared `design/`, a line per source with what it
  is good for, the kinds of product it fits and how it is reached. Fit is
  respected, by the product kinds of the `ui-product-kinds` skill — landing,
  web product, mobile app: marketing-site galleries are for landing pages,
  never for a dense dashboard, and a website is never the reference for a
  mobile app. The suggested starting list
  (`crates/core/skills/design-references/references/sources.md`) — public,
  generic, nothing paid:

- Carbon Design System — https://carbondesignsystem.com — good for: data tables, filters, dashboards, forms, notifications — fits: web product — access: public
- Polaris — https://polaris-react.shopify.com — good for: admin layouts, index tables, filters, empty states, settings — fits: web product — access: public
- Primer — https://primer.style — good for: navigation, lists, tables, forms, status — fits: web product (developer tools) — access: public
- Geist (Vercel) — https://vercel.com/geist — good for: dashboards, tables, navigation, empty states, type and spacing scales — fits: web product (developer tools) — access: public
- Atlassian Design System — https://atlassian.design — good for: navigation, tables, forms, messages — fits: web product — access: public
- shadcn/ui blocks — https://ui.shadcn.com/blocks — good for: dashboards, sidebars, KPI cards, charts, sign-in — fits: web product — access: public
- shadcn/ui registry — https://ui.shadcn.com/docs/components — good for: proven components to build a real page on (React, Tailwind) — fits: web product, landing — access: public — kind: component registry
- 21st.dev — https://21st.dev — good for: community components and blocks for React, Tailwind and shadcn/ui — fits: web product, landing — access: public, or mcp:<its official server> (the free tier limits copies a day) — kind: component registry
- Material Design 3 — https://m3.material.io — good for: components, navigation, states, motion — fits: mobile app (Android), web product — access: public
- Apple Human Interface Guidelines — https://developer.apple.com/design/human-interface-guidelines — good for: platform patterns, navigation, sheets, settings — fits: mobile app (iOS), native desktop apps — access: public
- Mobbin — https://mobbin.com — good for: real apps' flows and screens — onboarding, settings, paywalls, checkout — fits: mobile app, web product — access: mcp:<its official server>, or browser with the person's own sign-in and plan
- GOV.UK Design System — https://design-system.service.gov.uk — good for: forms, one question per page, error messages, plain content — fits: web product (forms, public services), any (content) — access: public
- Web Interface Guidelines (Vercel) — https://github.com/vercel-labs/web-interface-guidelines — good for: interaction, forms, motion, content and performance details — fits: web product, landing (writing, not pictures) — access: public
- Nielsen Norman Group articles — https://www.nngroup.com/articles — good for: why a pattern works, interaction details, states — fits: any (writing, not pictures) — access: public
- Awwwards — https://www.awwwards.com — good for: landing pages, art direction, motion — fits: landing only — access: browser
- Lapa Ninja — https://www.lapa.ninja — good for: landing pages, hero sections, pricing — fits: landing only — access: browser
- Land-book — https://land-book.com — good for: landing pages, page sections — fits: landing only — access: browser

- **The quality bar.** Every judgment of a screen — the designer's own, the
  critic's (`equipe design critique … --reference`), the second look's
  (`--reference`) and design QA's — puts it beside its patterns' references at
  the same size and answers: same quality tier? Where not, it names concrete
  differences (density, type scale, spacing rhythm, borders and surfaces,
  information said twice, primary-action clarity, alignment). The taste
  checklist: one primary action per view; no card inside a card; at most two
  levels of borders or surfaces; no information said twice; two or three type
  sizes per block; one spacing scale; tabular figures; the width used
  sensibly. Countable parts are counted — design QA's precheck reports
  `nested-cards`, `border-levels`, `type-sizes` (per block), `duplicate-text`
  and `primary-actions` as polish, and `lint --render` notes `type-sizes` (per
  screen) and `border-levels`; the rest is judged. Below the bar, the critic
  says `revise`, the second look `fix`; in design QA a gap the build opened
  against its spec is a blocker, and one the spec itself has is polish plus one
  design ticket to raise it.
- **Options worth a choice.** Options put to the person grow from named
  reference families and differ in concept (layout model, density, hierarchy);
  two strong beat three similar, and the critic rejects a near-duplicate set.
  Each is built as a real page on the project's own component library (or a
  proven open one for its stack) under `design/directions/<name>/`, starting
  from a proven component in a component registry where the stack has one
  (through the registry's official MCP server or API when the squad connected
  it), restyled to the project's tokens, its source and licence noted; never a
  premium component the squad hasn't licensed.
- **Locked, and reset once.** A system the person accepted is locked and
  day-to-day tickets build inside it. A foundation that keeps screens below
  their references (type scale, spacing, surfaces, colour, density) can be
  reset once: the designer proposes it as the person's decision, with the
  references as evidence; once chosen it is a design system ticket of its own,
  and the new system is locked again.
- **Small changes stay small.** A tweak to an existing screen is one version,
  compared with its references and iterated by the designer, then lint and the
  second look (its critic); no directions, comparison sheet, brief or decision
  card unless it holds a taste call. In `risk_batch` design QA a spec'd change
  of one screen with nothing new goes to the daily review; new screens and
  wide changes keep their own design QA.

### The UI craft library — the `ui-*` skills

The designer's craft rules live in skills it opens on demand
(`crates/core/skills/ui-*/`), each with concrete numbers, do and don't, and a
review checklist; the role text names them in one clause and the skill index
lists each in a line. They are equipe's own writing, drawing on public sources
cited in `crates/core/skills/ui-review/references/sources.md` (MIT-licensed
material adapted with attribution, see `THIRD_PARTY_NOTICES.md`).

| Skill | What it holds | Offered to |
|---|---|---|
| `ui-product-kinds` | Classify the work — landing, web product, mobile app — and take that kind's references, rules, widths and bar | designer |
| `ui-foundations` | Spacing scale, grid and width, density, type scale and setting, colour roles, contrast, dark mode, surfaces | designer |
| `ui-layout-and-states` | Hierarchy, one primary action, adding a part, progressive disclosure, web navigation, overlays, every state | designer |
| `ui-data-display` | Tables, KPI tiles, charts, filter bars, dashboards | designer |
| `ui-forms` | Structure, labels, the right control, validation and errors, submit, destructive actions | designer, frontend, mobile |
| `ui-mobile` | Platform conventions, navigation, touch and thumb reach, safe areas, onboarding, permissions, settings | designer, mobile |
| `ui-motion` | When to animate, durations, easing, press and hover, reduced motion | designer, frontend, mobile |
| `ui-accessibility` | WCAG 2.2 AA as concrete rules, semantics, how to check | designer, frontend, mobile |
| `ui-writing` | Labels, buttons, errors, empty states, confirmations, headings, writing for every language | designer |
| `ui-polish` | Focus and keyboard, loading and perceived speed, layout stability, edge-case content, details | designer, frontend, mobile |
| `ui-review` | Running the checklists against a rendered screen: kind, job walk, findings, fixes | designer |
| `ui-vocabulary` | A glossary of UI terms, one line each, to describe what you want | designer |

The kind checklist in `ui-review` is also said, word for word, to the critic
(`equipe design critique`), the second look and the light design QA session
(`design::critique::KIND_CHECKLIST`), so every judgment of a screen holds it
to its product kind's bar. An engine that can't open a file isn't sent the
library inline. To describe what you want, the vocabulary
(`crates/core/skills/ui-vocabulary/SKILL.md`) names the parts, styles and
behaviours the designer will use back.

### References and taste — `equipe design refs`

These are for brand work — a brand, a rebrand, a new visual direction — and
only brand work gets this full pass and its `design_tools` board tool; every
other ticket's board tools are as before (`members_without_phone` pins them). A ticket is brand work when its body has a line `Design: brand` (or
`Design: rebrand`, `Design: new direction`), the PM's explicit flag, or when
its title or body asks for one in so many words — a rebrand, a brand or visual
identity, a new visual direction (`design::brand::is_branding`). On such a
ticket the designer's first prompt carries the full pass
(`crates/core/assets/brand-pass.md`: brief, a board of five or more references
from two sources or more, two or three directions pictured, the critic, the
person's pick with a recommendation, the chosen tokens at AA, the build), and
its board server is started with `--branding`, which adds the `design_tools`
tool: `equipe design refs | palette | type | component` run for it outside
its engine's sandbox, in its worktree, reading only the listed sites.

References are studied for their patterns and layouts, never taken for a
brand: a reference's "what to learn" line that says to take another company's
logo, name, marks or words is refused (`refs::learn_problems`). Each reference
is kept in `design/references/` — `board.json` with its address, site, source,
date, aspects (pattern, layout, rhythm, type, colour) and the line, and its
pictures in `R<n>/` — and directions cite them (`direction propose --ref R2`).
`refs board` draws the contact sheet (`board.{html,png}`) and says what the
board still needs: five references or more, from two sources or more, each
pictured and said.

| Source | How it is read |
|---|---|
| Lapa Ninja, Mobbin | In the designer's own browser (the squad's), its screenshot recorded with `refs add`. Lapa Ninja shows automated readers a check; Mobbin needs the person's own sign-in and plan, and is offered only when the squad's browser holds one. |
| Fontpair, 21st.dev, Google Fonts, any public site | `refs shoot`: at most 8 pages a call at one or two widths, 5 s between two pages of one site, what robots.txt disallows left alone, Chrome's own user agent (nothing hides that it is automated). A page that asks whether a person is there is reported and its picture dropped — a tool never answers it. `refs search 21st <pattern>` lists a category's components from its markdown twin. |

Nothing here signs up, signs in, accepts terms, holds a key or gets round a
paywall or a check.

The scripts and the one npm package they import (`playwright`, no browser
download) live in `~/.equipe/design-kit`, set up on first use.

### Design checks — `equipe design lint`

The rules are data in `crates/core/src/design/lint/rules.rs`, each with an
id, a tier, what it checks and how to fix it (`equipe design lint --rules`).

- **Floor** — never waived: colours, type and spacing only from the system's
  tokens (a literal the system holds is a finishing note to name the token);
  focus that shows; a reduced-motion version of whatever moves; no invented
  proof (rounded audience figures, uptime, ratings, testimonials, logo walls,
  placeholder names, lorem ipsum — real figures carry `data-source`); every
  control named; and, drawn, no sideways overflow, no clipped text, text
  contrast, the `data-primary` action in the first screenful.
- **Habits** of generated screens — fixed, or kept with a reason written in
  `design/decisions/` as `Waive <id> [(screen)]: <reason>` (a floor rule's
  waiver is refused and said): a label over every heading, spaced capitals,
  one coloured word, a gradient first screen, cards in cards, emoji as
  icons, everything centred, one radius and shadow everywhere, decorative
  dots, a form that pretends to send.
- **Build from the system** — only where the project has one (a
  `registry.json` or `components.json` at or above the file, inside its
  repository): no value written out that a token holds (`system-token` —
  colours, sizes, radii, shadows), no raw element re-making a registry
  component (`system-component` — a styled `<button>` where there is Button, a
  `role="dialog"` where there is Dialog; which markup each component replaces
  is data in `crates/core/assets/design/components.json`), no import around the
  registry (`system-import` — Radix, another UI kit, the registry's source by
  path). Blocks like the floor, at edit time and at a hand-in; a real
  exception is a `Waive` line scoped to its file or screen. The registry's own
  components and the theme's stylesheet aren't held to it.
- **Finish** — said, never blocking: name the token, tabular figures, heading
  order, image size, line length, touch targets, more than six type sizes on a
  screen, boxes framed more than two deep.

The source is read loosely (CSS, HTML, JSX, Vue, Svelte). `--render` draws
the page in the Mac's Chrome at 360, 390, 768, 1024, 1280, 1440 and 1920:
`assets/design/lint.mjs` only measures, the core judges. A drawn check is
kept in `design/checks/<flow>/<screen>.json` with its pictures; the UI map
shows each screen's latest, and a member's conversation shows each check and
second look as a card.

On Claude Code, a designer's and a frontend engineer's skills plugin has a
`PostToolUse` hook: after each edit to a UI file the floor is read in what was
just written and handed back in the same turn. A designer's edits are also
written to `design/checks/pending.json`, and its hand-in is refused until each
was drawn and checked, passed the floor, and had a passing second look after
the check (a new screen's first screenful looked at first). The directions it
explored (`design/directions/`) are left to the directions' log: only what
ships waits for a check. A check is kept where the hand-in looks — the design
folder nearest the files checked — whichever system `--design` checked them
against; a refusal names each file, the rule and the exact commands, and says
when a check was kept in another folder. Refused the same way a second time,
the work goes in with what wasn't cleared marked in its hand-in for the person,
and the PM is told: a check that can't be cleared is theirs to see, not a loop.

The second look asks only for facts the brief, the project's files or the
design system give. A fact none of them has is an open question with neutral
wording to use meanwhile, never a fix: it doesn't hold a pass, and the designer
uses the wording or asks with `ask_decision` while it carries on. After three
looks in a row that ask for fixes (the first screenful's or the whole's), the
hand-in goes with what the last look asked for marked for the person, and the
PM is told — the same way as a refusal met twice.

A member's commands carry `EQUIPE_MEMBER_ENGINE` (`claude` or `codex`), so
`second-look` and `critique` know the caller's engine even for a command run
outside its engine's sandbox, where the engine's own variables are gone; with
no engine known and none named, they say which they took. When the sandbox
has no network for the engine, the member is told to run the command again
outside it, or, if that isn't allowed, to say so in its hand-in rather than
try again.

### A component system — `equipe design system | matrix | registry`

`crates/core/src/design/system/` reads a project's design system as code,
wherever it keeps it: tokens as DTCG JSON, as CSS variables (`:root`,
`.dark`, `[data-theme=dark]`, `prefers-color-scheme`) or as a Tailwind v4
`@theme`; `components.json`; `registry.json` and what `shadcn build` wrote;
the components' folder (by `components.json`'s aliases); and a docs page per
component (a `docs/` route or file, or a story).

- `equipe design system` says what is there and missing, the coverage against
  the catalog's full set ("47/49 components, missing: …") and each token text
  pair under AA in light or dark.
- `equipe design matrix <component> --url <docs page, {name} for each>` puts
  every `[data-example]` on the page in each state its pattern has, light and
  dark, at the widths asked, into a contact sheet a width
  (`assets/design/matrix.mjs` measures; `system/matrix.rs` judges); runs
  axe-core's WCAG A/AA rules (installed once into the design kit — a run
  without it says so) and a keyboard pass. Kept in
  `design/checks/components/<name>.json`.
- `equipe design registry [--build] [--try <item>] [--npx]` checks each item
  (files, declared dependencies, `registryDependencies` that install from this
  registry rather than shadcn's, `meta.source` and `meta.license`, no other
  brand's pictures), the build against the source, and installs an item into a
  scratch project offline from the build; `--npx` also runs the real
  `npx shadcn add`.

What equipe knows of each component — its WAI-ARIA pattern, the part an
example holds, its states, its keyboard checks, how it opens, the markup it
replaces — is the data file `crates/core/assets/design/components.json`:
extend it there, not in code. A designer's and a frontend engineer's board
server runs these three for them as `system_tools`, outside the engine's
sandbox, in the member's worktree.

Who reviews is the squad's setting (Settings → Engines → Design reviews, or the
PM's `change_setting` `design_reviewer`): **auto** (the default) takes the
other engine when it is installed, signed in and its plan has room — its week
under 90% and not on course to run out before it resets, its five hours under
95% — and otherwise a fresh session of the member's own engine; **same** is
always the member's own, fresh; **other** always the other one. What matters is
a reviewer that saw none of the work being made, not the vendor, so a person on
only Claude or only Codex always gets that engine, and when the other engine is
tried and can't run, the member's own takes over. The daemon decides at each of
a member's turns and passes it in `EQUIPE_DESIGN_REVIEWER`, which stays outside
the sandbox too; from a person's own terminal the command decides by what is
installed. A critique is kept with the engine that wrote it and why
(`<!-- equipe-reviewer: … -->`, its first line), a second look's verdict with
`reviewer`, and the designer studio shows both.

## Remixing references — the `design` tools (equipe-design)

[equipe-design](https://github.com/equipe-labs/equipe-design) extracts a design
system from a site's public pages and remixes it with the person's own brand.
Its `equipe-design mcp` command is an MCP server, `design`, with
`design_extract`, `design_report`, `design_remix` and `design_emit` (and the
design library's `design_library_*`).

**How it connects.** Each turn the daemon looks for an `equipe-design`
executable on its own PATH (the service's PATH, `daemon.rs`
`service_path`). Found, and the member's role is `designer` with equipe's board
tools, the turn's MCP config gets

```json
"design": { "type": "stdio", "command": "/opt/homebrew/bin/equipe-design", "args": ["mcp"] }
```

beside `equipe`, `browser` and `desktop` (`runtime.rs`, `DESIGN_SERVER`; on
Codex the same server through its own config). The tools are
`mcp__design__design_extract` and so on; in Auto and Full they don't ask, in
Ask each call comes to the person. No other role gets it. Not on PATH: no
`design` server and no word of it to the designer (its `design-remix` skill isn't offered) — equipe never
installs it (until it is published, the person links a checkout onto PATH:
`npm link` in its `packages/cli`). Since the PATH is searched each turn, an install shows up on the next
turn without restarting the daemon.

**Its own browser.** The engine launches a headless Chrome of its own, with a
fresh temporary profile. It never attaches to the member's `browser` (the
window the person's and the squad's sign-ins live in), carries no cookies, and
stops at robots.txt or a sign-in wall.

**The flow** (the designer's `design-remix` skill):

1. `design_extract` from two or three reference URLs into the ticket's `outputs/…/extract/<host>/`.
2. Tell the person in a few lines what `design_report` says it couldn't see.
3. `design_remix` with the person's brand colour and name → variants, each with a sheet PNG.
4. Put the sheets to the person on `ask_decision`.
5. `design_emit` the chosen one with `dtcgLegacy: true` into the project's `design/`.
6. `equipe design check`, then hand in with the pictures.
