# The design system

A squad's design system is a folder its designer keeps: the colours, type,
spacing, radius, shadow and motion of what the squad makes, its rules in
prose, and its logos, icons and pictures. equipe shows it on the desktop
app's **디자인 시스템** screen (the sidebar, or `/design`), the terminal
summarises it (`/design`), and a phone can read it — always through the
daemon, so a screen on another Mac or the phone never reads the daemon's disk.

## Where it lives

| Whose | Folder |
|---|---|
| A project's own | `<repository>/design/` — its tokens, components, brand and mockups, versioned with its code |
| The squad's | `~/.equipe/squads/<squad>/home/design/` — only the principles every project shares |
| A draft | `design/` in the worktree of a ticket **in review** — shown as "검토 중 · T-n" until the person accepts it |

A project's design assets are made in its repository from the start, never
drafted in the squad's folder and carried over later. The designer's work on
them is a `code` ticket on that repository, and on it the designer changes
only `design/`: `submit_for_review` refuses a designer's code-ticket hand-in
whose worktree changes anything outside it — committed or not, new files
included — and names the files, to undo and propose to an engineer
(`runtime::design_qa`). Applying the system to screens is an engineer's
ticket. The designer's own code ticket skips design QA. A task ticket still
lands the squad's `design/` on acceptance, for the shared principles.

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
- **Where a token shows**: colours under 색; type styles, families,
  weights and type sizes under 타이포그래피; dimensions named `radius`/
  `corner` under 모서리, the rest under 간격; shadows under 그림자;
  durations, curves and transitions under 모션.
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

- **개요**: the logos on the system's own light and dark backgrounds, the lead
  colours (brand, accent, background, text), a type sample, and how much is in
  each section. Problems reading `tokens.json` are listed here.
- **색**: swatches by group, light and dark side by side, each with its hex
  and WCAG 2.x contrast (AAA 7:1, AA 4.5:1, 큰 글자 3:1) against the
  system's background — or, for a surface, its text. A click on a colour
  copies its hex; on a name, `{the.token}`.
- **타이포그래피**: each style set live in its font, size, weight and line
  height, in the person's language and English. A font this Mac doesn't have
  says so and names the one it is shown in.
- **간격 · 모서리 · 그림자 · 모션**: specimens at real size; a motion token
  plays its duration and curve under the pointer.
- **에셋**: pictures on a checkerboard, big in a lightbox (←/→, Esc), saved
  to Downloads (from any Mac) or shown in Finder (this Mac).
- **가이드라인**: `guidelines.md`, with its `##`/`###` headings as contents.
- **바뀐 것**: the tokens added, removed or changed since the person last
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

See `docs/remote-protocol.md` for the phone's side.
