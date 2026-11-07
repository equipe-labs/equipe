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
| The squad's | `~/.equipe/squads/<squad>/home/design/` — `design/` at the top of the squad's folder |
| A project's own | `<repository>/design/` — when a project looks different from the rest of the squad |
| A draft | `design/` in the worktree of a ticket **in review** — shown as "In review · T-n" until the person accepts it |

The squad's folder is where the designer starts and where task tickets'
work lands when the person accepts it, so a designer's brand or design task
updates `design/` in its worktree and the change becomes the squad's on
acceptance. A code ticket on a project with its own look changes that
repository's `design/` the same way.

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
  components/          a rendered preview (PNG or SVG) of each key component
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
  `problems`, `guidelines`, `assets` (`{ path, name, kind, size }`) and a
  `stamp`.
- While any screen follows the squad, the daemon looks at the folders every
  1.5 s — their files' sizes and times only — and sends the event again when
  one changed, so the screen follows what the designer saves.
- `read_image` `{ squad, path, max? }` for each asset.
- The designer studio (below) asks `design_studio` `{ squad, ticket }` →
  `Studio` (`crates/protocol/src/studio.rs`) every 2 s while it is open: what
  the ticket's worktree design folder holds — each screen's latest check and
  second look, the edits not checked yet, the directions' log, the critiques,
  the reference board and other pictures. Read-only, from inside that folder
  only (links out are not followed), every file and list capped
  (`crates/core/src/design/studio.rs`); its pictures through `read_image`.
  The read-only real view may ask it; phones are not routed it.

See `docs/remote-protocol.md` for the phone's side.

### The designer studio (desktop)

Watch a ticket's design work as it happens (`crates/desktop/src/views/studio.rs`;
from a ticket's card, the member's lane in Orchestration, or the conversation's
header when the member is on a design ticket). Under the title, an honest status
line ("Working · 12m", "Waiting for you: pick a direction", "Checks failing:
Contrast — …", "Handed in"); the steps — brief, references, directions, build,
checks, hand-in — each lit from what the member did, with how much and when; the
latest picture it drew, with the seven widths a check drew it at (or the
directions side by side while a pick waits); and a timeline of what it left —
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
| `second-look <flow/screen> [--first] [--criteria …]` | A fresh, read-only reviewer judges the checked pictures, given only them, the brief and criteria, and the system's guidelines and tokens; its verdict is kept on the screen's record. `--prompt` / `--answer <file>` for a fresh subagent when no engine can run. |
| `direction propose\|choose\|list` | The directions proposed and chosen (`design/directions/log.json`) by layout, type, palette, density and motion; a proposal that repeats a recent chosen one in every dimension the brand leaves free, or differs from its fellow options only in colour, is refused. |
| `kept record\|check <files\|url> --screen …` | A screen's links, fields, actions and headings, recorded before a redesign (`design/kept/`) and checked after; a removal only where the brief or a decision says `Remove <kind>: <item>`. |

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
- **Finish** — said, never blocking: name the token, tabular figures, heading
  order, image size, line length, touch targets.

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
