# equipe design guidelines

The design rules for equipe (CLI·daemon·desktop). Values are in `tokens.json` (a generated file — made from `tokens/`, see `README.md`), component rules in `components/components.md`, the logo in `assets/logo/`. The squad's shared principles are in the squad folder's `design/principles.md` and aren't copied here.

Source: the brand part (§0–§7) is carried over as is from the T-144 brand guidelines v1 (settled 2026-10-16). The pictures and evidence files the text mentions (`compare-color.png`, `compare-type.png`, `type-8lang.png`, `audit.md`, `brand.md`, `contrast.md`) are in `brand/`, as are `logo-usage.png`·`logo-in-use.png`. The drafts from before the choice (`options/`) and `brand-tokens.draft.json` are only in the squad folder's `outputs/T-144 brand guidelines/`.

## Design system principles (higher wins in a conflict)
1. **The person's approval shows the most.** The decision card stands out most on any screen, and an area has one primary action.
2. **Never say it with colour alone.** A status is colour + symbol + words. Never below 4.5:1 for text · 3:1 for graphics.
3. **True to each platform, one name.** iOS·Android follow the system font·Dynamic Type·sheet conventions, but the same role uses the same token name (`radius.card`), with value differences written in the token's `$extensions["im.equipe"].platform`.
4. **Motion is short, one curve, and respects reduce motion.**
5. **An existing token before a new value.** When one feels needed, first write down "why no existing token will do" (`README.md` governance).

## How engineers use it
- Product code uses semantic and component tokens only. Never `color.raw.*` or hex·px literals directly.
- A value you need that isn't there is a design request on a ticket — don't make a new constant in your own code.
- `tokens.json` and the files under `build/` are **generated, never edited by hand.** Edit `tokens/` and run `npm run check`.
- A change that changes a screen goes through the designer's design QA.

## Brand 0. What the person chose (2026-10-16, every recommendation)

| # | Question | Options (recommended first) |
|---|---|---|
| D1 | **Logo master** — the site (four blue dots) and the app·store (`e.` with a grey dot) differ | **①** `e.` + a round blue dot `#3F6FF5` (= option C of T-105; the app·store only colour the dot, the site only swaps the logo) · ② `e.` with the grey dot as is (the app today; the site follows) · ③ the four blue dots as the main logo (every app icon redone) |
| D2 | **Colour option** (`compare-color.png`) | **A Signal blue** (warm neutrals + one blue dot, links `#1D4ED8`) · B Steel chrome (the desktop's current steel blue) · C Mono ink (no accent colour, a grey dot) |
| D3 | **Typeface option** (`compare-type.png`) | **1 Inter + a family for each script** (Inter for web·marketing, the system font in the apps; Pretendard for Korean, Noto Sans for Japanese·Chinese) · 2 system fonts only · 3 IBM Plex Sans |
| D4 | **Register in each language** — decided first for Korean | **Korean: the polite -yo register (haeyo-che) throughout** · Korean: the formal register (hapsyo-che) throughout |

## Brand 1. Logo

### 1.1 Construction (per D1①)

- **Mark `e.`**: a void `#0A0A0C` tile + a bone `#EFEDE7` lowercase `e` + the brand dot. App icon·favicon·avatar. Corner radius = 22% of a side (the same as the site tile's 14/64).
- **Wordmark `equipe.`**: lowercase + dot. Helvetica Neue Bold, tracking −0.04em (BRAND.md's −6px@150px). The dot is a **circle** (diameter ≈ 0.115em), colour `brand.dot` `#3F6FF5`.
- **What the dot means**: the site's four dots' "filled dot = the person who leads", reduced to one dot (carrying on the meaning in `equipe-site/design/tokens.md`, T-105). The four dots leave the logo and stay as an illustration in the body.
- Pictures: `logo-usage.png` ①, in use: `logo-in-use.png`.

### 1.2 How the name is written (proposed)
- In sentences·UI·meta: **`equipe`** — always lowercase, no dot (as in glossary §1). Lowercase at the start of a sentence too.
- The dot is for the **logo (the picture)** only. Never write `equipe.` as text (Android's `wordmark` string `equipe.` stands in for the logo, so it's the exception).
- The iOS home-screen name is `equipe` (`Equipe` at `project.yml:61` is to be fixed — `:96` of the same file already says `equipe`).
- In pt-BR·fr the first mention gets a descriptor ("equipe, um squad…") — exactly as glossary §1.

### 1.3 Usage rules (`logo-usage.png`)
- **Clear space**: x = ¼ of the mark's side, for the wordmark ½ of the `e`'s height — nothing comes inside.
- **Minimum size**: mark 16px, wordmark letter height 16px (≈ 72px wide). At 16px the dot still keeps 2px or more (checked by rendering). Print 8mm / 20mm wide is **a proposal (not checked in print)**.
- **Backgrounds**: on light backgrounds the letters are `#15161A`, on dark ones bone `#EFEDE7`. The dot is `#3F6FF5` on both (contrast §2.4: light `bg` 4.01:1, void 4.52:1, white 4.37:1 — above the 3:1 graphics threshold).
- **One-colour version**: one-colour print, Android themed icons, iOS tint → the dot takes the same single colour as the letters.
- **Don't**: change the dot's or letters' colour, stretch·slant, add shadows·gradients, set it on a blue background·a photo, use another font, replace the `e` with the four dots (the last is the site's current logo — dropped by D1①).

### 1.4 Relation to the app icon
- App icon = the mark tile filling the whole face (1024²). The OS cuts the corners (iOS mask, Android adaptive icon — foreground layer `e.`, background void).
- Dark/tinted variants: the dot goes one colour (as T-72·T-88's tint). In light and dark alike the tile is void.
- The GitHub organisation avatar keeps BRAND.md's "label style" (bone background + black `e` + dot), with the dot's colour per D1 — meant to stand out on dark and light themes alike (BRAND.md Assets).
- **No longer font-dependent (T-178, 2026-10-31)**: the master SVGs now draw the `e` as an outline path with no text element (`assets/logo/mark.svg`, the light tile `mark-light.svg`, the foreground without a tile `mark-glyph.svg`, the wordmarks `wordmark-light.svg`·`wordmark-on-dark.svg`). The outlines come from **TeX Gyre Heros Bold** (a free Helvetica-style font, [GUST Font License](https://www.gust.org.pl/projects/e-foundry/licenses) — use·modification·redistribution allowed; a rename is required only when distributing a modified font file; copy at `assets/logo/GUST-FONT-LICENSE.txt`). Laid over the old Helvetica Neue render, the difference stays within 1–2% at the outline's edge (`brand/logo-outline-compare.png`). Helvetica Neue outlines are **not used**: the font software bundled with the Mac belongs to Linotype/Monotype·Apple (checked in the font files' copyright notices) and no public document was found allowing outline conversion and redistribution for a logo — when unsure, the rule is not to use it (unconfirmed; using it would need a Monotype logo licence enquiry). Inter Bold (OFL) was tried too, but its `e` tail is cut on a slant and it's larger, visibly different, so it wasn't adopted.

### 1.5 Assets
| File | Status |
|---|---|
| Mark SVG (outlined) `mark.svg`·`mark-light.svg`·`mark-glyph.svg`, wordmarks `wordmark-light.svg`·`wordmark-on-dark.svg` | **Present** (T-178, outlined·based on Heros) — no mono wordmark yet |
| App icon PNG 1024 (light·dark·tinted), Mac `.icns` | In T-72·T-88 (only the dot's colour·shape re-rendered) |
| Site `favicon.svg/.ico`, `apple-touch-icon.png`, `og.png`, `logo.png` | Present — the mark needs replacing (exactly T-105's "files to change" list) |
| Drafts (this folder) | `logo-usage.png`, `logo-in-use.png` |

## Brand 2. Colour

### 2.1 Principles
1. **Only two kinds of colour: the brand blue (just one) and the semantic colours.** Everything else is warm neutrals (bone·void).
2. Light and dark use the same **role names**. Only the values differ per mode.
3. Text pairs meet WCAG 2.x AA (4.5:1), graphics (focus ring·dot·icons) 3:1 or more. A status is never told by colour alone (words·icons with it).
4. BRAND.md's "no second accent colour" becomes "**one blue** is the only colour (+ semantic colours)" (per D2 A·B. C keeps BRAND.md as is).

### 2.2 Role tokens (2.0.0 — the desktop palette as shipped, T-699)

T-144 chose A (one blue, warm neutrals). The desktop then re-tuned that palette on screen (a lifted dark page, one
warm neutral, a calmer blue), and tokens 2.0.0 take those values as the reference. Full list and the map to
`theme.rs`: `docs/t699-palette-map.md`; every value lives in `tokens/semantic.color.{light,dark}.json`.

| Role | Light | Dark | Use |
|---|---|---|---|
| `bg` | `#FAF9F6` | `#151513` | page·app background |
| `side` (hover · selected) | `#F3F2EF` (`#EEEDE8` · `#E7E6E1`) | `#100F0E` (`#1C1C1A` · `#272624`) | desktop sidebar, solid row fills |
| `surface` · `raised` | `#FFFFFF` | `#1F1E1C` | cards·sheets·fields |
| `popover` (hover) | `#FFFFFF` (`#F2F1EE`) | `#252422` (`#2F2E2B`) | menus·popovers, solid |
| `sunken` | `#F2F1ED` | `#1B1A19` | code·pressed surfaces |
| `text` | `#181713` | `#F3F0E9` | titles·controls·body |
| `prose` | `#24231F` | `#E5E2DB` | reading text — a shade softer than `text` |
| `text-soft` | `#45443E` | `#CCC8C2` | secondary text a step above `muted` |
| `muted` | `#62605A` | `#A2A09A` | secondary text·meta |
| `accent` · `focus` | `#2160AB` | `#7EB8F3` | links·text buttons·selection·focus |
| `onAccent` | `#FAF9F6` | `#151513` | text on a filled accent surface |
| `action` (hover · pressed) | `#181713` (`#2C2B27` · `#050504`) | `#ECEAE4` (`#FFFFFF` · `#D6D3CC`) | the main button: the text colour inverted |
| `dot` | `#3F6FF5` | `#3F6FF5` | **the brand dot** (a logo-only graphic, not a text colour) |
| `ok` / `work` / `wait` / `err` | `#1F6B33` / `#865901` / `#B83A0B` / `#AA2A27` | `#7FCB8E` / `#EABA67` / `#FF7A45` / `#F2857D` | done / in progress / your turn·waiting for permission / error |
| `errFill` (hover · pressed) | `#EEE0DE` (`#EBD9D7` · `#E9D2CF`) | `#352728` (`#3C2A2B` · `#442E2E`) | opaque fill behind `err` text — the soft destructive button (Deny) |
| `tooltip` | `#232220` | `#2F2E2C` | a small dark chip in either mode; text `#F3F0E9`, hint `#A9A7A1` |
| `line` | `rgba(0,0,0,.10)` | `rgba(255,255,255,.10)` | hairline (decoration — not subject to text contrast) |
| `member-0…7` | OKLCH 0.49 0.12 | OKLCH 0.80 0.11 | member·project colours, one lightness so no member is louder |

- Tints (`accent-bg`, `ok-bg`, `work-bg`, `wait-bg`, `err-bg`) are the colour at 8 % in light and 12 % in dark.
- **`errFill` is opaque on purpose (T-492)**: a translucent tint over a translucent bar or sheet changes with whatever scrolls behind it, and at 8% over iOS's `.bar` the Deny label fell to 4.27:1 (T-491). The soft destructive button takes `errFill` with `err` text, and for hover·pressed its own opaque steps instead of the `hover`·`pressed` overlays. `err-bg` stays for badges·banners on solid surfaces.
- **Filled primary button**: the marketing web (the site) uses an `accent` fill + `onAccent` text. **Product UI (app·desktop) uses the `action` fill** (the text colour inverted), with blue only for links·selection·focus·the dot, so the one important action, like "approve", isn't lost among blue buttons.
- Member colours are *a system for telling members apart*, not brand colours; they meet 4.5:1 as words on the page and on the selected sidebar row (`docs/contrast.md`).

### 2.3 The three options compared (`compare-color.png`)

T-144's comparison, kept as the record of why A; its values are the 1.x ones, superseded by §2.2.

| | **A Signal blue (recommended)** | B Steel chrome | C Mono ink |
|---|---|---|---|
| In a line | warm neutrals + one blue dot·links | the desktop today: chrome-grey dot·steel-blue links | no colour; the ink is the emphasis |
| `accent` light/dark | `#1D4ED8` / `#8FB0FF` | `#2E5C8A` / `#B4CDE6` | `#0A0A0C` / `#EFEDE7` |
| Dot | `#3F6FF5` circle | `#7F8591`/`#8A909B` grey (square possible) | `#5B5E66`/`#7C7E85` grey |
| Pros | keeps the site's current link·button colours (least change), the dot reads in a 16px favicon, one line of BRAND.md changes | fits the desktop·BRAND.md best, calm | most restrained, the same as the app's current tokens |
| Cons | the app·desktop accent changes (`#2E5C8A`→`#1D4ED8`) | the site's whole blue UI changes; the dot doesn't show, so the logo has less character | links rely on underlines alone; with a grey dot the "person who leads" meaning is lost; weak hierarchy beyond the status colours |
| Where it changes | desktop accent, app links·focus, the icon's dot | the whole site CSS | the whole site CSS |
| Below contrast | none | none | none |

Why A: ① it uses the site's values as they are (already AA-checked and documented, `tokens.md`), so the site barely changes ② the desktop·app accent sits in small places — *links·selection·focus* — so changing it is low risk ③ the blue dot keeps the site's meaning, "the person who leads", in the logo. (The judgement is a proposal — taste weighs heavily.)

### 2.4 Contrast
Every pair is computed from the tokens on each `npm run check` and written to `docs/contrast.md` (206 pairs, light and
dark, 0 below threshold at 2.0.0); the build stops if one fails. The table that stood here measured the T-144
values. One known gap: the switch's off track is under 3:1 (`docs/t699-palette-map.md`, "Open").

## Brand 3. Typefaces

### 3.1 The choice (`compare-type.png`)
| | **1 Inter + a family for each script (recommended)** | 2 system fonts only | 3 IBM Plex Sans |
|---|---|---|---|
| Web·marketing | Inter (self-hosted — the site today); per script: ko Pretendard, ja/zh Noto Sans | `system-ui` | Plex Sans + Plex KR + Noto |
| App body | the platform's system font (SF Pro·Roboto) | same | same (needs bundling) or the system font |
| Pros | no change to the site; each script's family sits close to Inter in weight·tone (checked for Korean with Pretendard); all open licences | nothing bundled, the platform's feel | character |
| Cons | the size of CJK web fonts (needs subsetting), bringing in Pretendard | the web looks different on each OS, weaker brand consistency | Japanese·Chinese fall back to other fonts (unconfirmed), three bundles |
| Licence | Inter·Pretendard·Noto: SIL OFL 1.1 (**check the licence files before shipping — the originals weren't checked for this document**) | provided by the OS | Plex: OFL (unconfirmed) |

### 3.2 Stacks (recommended 1)
- **Web**: `--sans: 'Inter Variable', Inter, system-ui, sans-serif` + per-language `:lang()` rules — the site's `global.css:197-204` already has ko·ja·zh-CN. To add: for ko, `'Pretendard Variable'` after Inter and before `Apple SD Gothic Neo` (proposed); a new `:lang(zh-TW)` rule for zh-Hant (the site has no zh-Hant yet). Every further language gets its own `:lang()` rule with a family made for its script.
- **Mono**: `JetBrains Mono Variable`, falling back to `ui-monospace, Menlo`. The desktop's `Menlo` stays as is (platform).
- **iOS**: SF Pro (the system) — following Dynamic Type. **Android**: Roboto/the system. No brand font is bundled in the apps (CLAUDE.md "each platform's conventions" + the 1-second cold-start requirement — avoiding font loading cost, proposed).
- **Logo**: an outlined SVG whatever the font (§1.4).
- Weights: titles 700 (tracking −0.02em, 0 for CJK), body 400, buttons·labels 600. The scales stay as they are: site H1 56/1.05 (mobile 36), H2 32, body 18/1.6 (mobile 17) — source `tokens.md`. Desktop 15/25·14·13·12 — `theme.rs`. Unifying the two is in the design system ticket's scope.

### 3.3 Fallbacks for the 8 languages (render checked in `type-8lang.png`)
| Language | Web stack | Apple apps | Android apps | Notes |
|---|---|---|---|---|
| en | Inter | SF Pro | Roboto | |
| ko | Pretendard → Inter | SF + Apple SD Gothic Neo | Roboto + Noto Sans KR | `word-break: keep-all`, body line height 1.65 (the desktop's value) |
| ja | Inter → Hiragino Sans → Noto Sans JP | SF + Hiragino Sans | Noto Sans JP | `line-break: strict`, titles `auto-phrase` — the site today |
| zh-CN (zh-Hans) | Inter → PingFang SC → Noto Sans SC | SF + PingFang SC | Noto Sans SC | |
| zh-Hant | Inter → PingFang TC → Noto Sans TC | SF + PingFang TC | Noto Sans TC | only on the desktop; the rendered line is a new translation, **not reviewed** |
| es · pt-BR · de · fr | Inter | SF Pro | Roboto | long-word (de) wrapping: `hyphens: auto` recommended (proposed, untested) |
- The platform columns name each OS's default fonts and weren't rendered on this sheet (only the web stacks were rendered: `type-8lang.png`, Chrome + Google Fonts). The Android column is **unconfirmed** (Roboto·Noto assumed).

## Brand 4. Icons·illustration·screenshots (proposed — rules only, sample sheet to follow)

**Icons**
- Follow the platform's set: desktop·web **Lucide** (today, `icons.rs`), iOS **SF Symbols**, Android **Material Symbols**. No new brand icon set.
- Outline style, stroke 2px@24 (1.5px at 16px), round caps·corners, following the text colour (`text`/`muted`). Colour is only for the status icons (`ok/work/wait/err`). No filled multicolour·gradients·emoji.
- Two sizes: 16·20 (desktop `ICON_SM`/`ICON`), touch area 44pt/48dp.

**Illustration**
- If any, **dots and hairlines** only: the site's 2×2 dot motif (one filled dot = the person) carries on as the illustration language. No people·mascots·3D·gradients. Blue only for the one "leading dot".
- BRAND.md's chrome objects·grain are **limited to brand surfaces** like social cards·avatars, never product screens.

**Screenshots**
- **Real screens only** (avoid placeholder frames·fake data — in line with the store's 2.3 rule too). The current T-72 store screenshots are placeholders, so replacing them with real screens comes first.
- A light·dark pair, background `bg` (light) / void (dark), thin device frames. Captions are one line, a sentence, in the language's own register (Korean: haeyo-che; English: sentence case), **no ALL CAPS decoration** (`APPROVE` and the like).
- Personal information·tokens·paths are hidden.

## Brand 5. Voice in short, and the canonical copy

The full rules·examples·don'ts are in `brand.md` §3. Here only what implementers use directly.

### 5.1 The one-line rule
Short·active·concrete. What the person does comes first. No hype words·exclamation marks·emoji·ALL CAPS decoration. The name is `equipe` (lowercase, no dot).

### 5.2 Each language
Every language equipe speaks has one register throughout, chosen for that language and written in `i18n/glossary.md`, with the glossary's terms and spelling. English: plain sentence case. Korean (D4): the **polite -yo register (haeyo-che)**; "you" (dangsin) only in introductory copy, the app UI drops the subject; formal-register (hapsyo-che) strings (about 235 desktop · 45 iOS · 43 Android, audit V3) go to a translation-review ticket. A language without a decided register gets one in the glossary before it is translated.

### 5.3 Copy that needs a canonical version (the person decides → fixed in one place)
- **Main headline**: ① "Lead a team of AI agents. You approve every merge." (live) ② "Your AI software squad, led by you." (copy.md). Recommended **①** — the action (leading) and the guarantee (approval) are in one sentence (proposed).
- **One-line description (store subtitle·meta)**: one sentence along the lines of `brand.md` §1's "an AI software squad led by one person", used everywhere.

## Brand 6. Hand-off to the design system ticket — token names and where values move

Token names are `color.<role>` (values per mode). Draft JSON: `brand-tokens.draft.json` (DTCG, recommended A).

| Token | Site now → proposed | Desktop now → proposed | App (`tokens.json`) now → proposed |
|---|---|---|---|
| `bg` | `#FAF8F4`→`#F6F5F1` / `#121211`→`#0A0A0C` | same (no change) | same |
| `surface` | `#F0EDE6`→`#FFFFFF` / `#1B1B19`→`#16171A` | `raised` `#FFFFFF`/`#141417`→`#16171A` | same |
| `sunken` | `--code-bg` `#EAE6DD`→`#EAE7E0` / `#262623`→`#111114` | `code_bg` `#ECEBE6`→`#EAE7E0` / `#111114` | (none → new) |
| `text` | `#1A1A17`→`#15161A` / `#F2EFE8`→`#EFEDE7` | `#15161A` / `#EFEDE7` same | `#0A0A0C`→`#15161A` / same |
| `muted` | `#57544C`→`#4F5159` / `#A9A59B`→`#A3A5AB` | `#5E6068`→`#4F5159` / `#8A8C93`→`#A3A5AB` | merges `textSecondary` |
| `accent` | `#1D4ED8`/`#8FB0FF` same | `#2E5C8A`→`#1D4ED8` / `#B4CDE6`→`#8FB0FF` | (new: links·focus) |
| `dot` | (new) | (new) | (new) |
| `ok/work/wait/err` | (new) | `err` `#AD2E27`/`#EE7B74` same, `work` `#825600`/`#E2B865`·`wait` `#A04610`/`#EC9A62` same, `ok` new | `danger`→`err`; the rest new |

Note: this table is a **list** of where values change. Each change is a screen change, so designer QA is required (squad memory `design-system-and-brand`). Unifying spacing·radii (site 8/14, app 12/16/24) is outside this ticket — it belongs to the design system ticket.

## Brand 7. Open · unconfirmed

1. **The redistribution licence for a Helvetica Neue outlined logo** is unconfirmed — before making the masters `mark.svg`·`wordmark.svg`, decide between (a) confirming the Apple/Linotype licence, or (b) redrawing the `e` from an openly licensed font (e.g. Inter Bold) to match the current shape.
2. **The Pretendard·Inter·Noto licence texts** weren't opened while writing this (known to be OFL — check before shipping).
3. Contrast was newly computed for the three palette options only; **the current site·desktop values weren't re-measured**. The 8 member avatar colours' contrast isn't computed.
4. The new colours·typefaces **weren't rendered on real iOS·Android screens** (the sheets are web renders). To check at the design QA stage.
5. The print minimum size (8mm/20mm wide) is a proposed value, not checked in print.
6. The zh-Hant sample sentence is a new translation, not yet reviewed.
7. The mismatch in language lists (pt-BR ↔ zh-Hant) is a product decision, handed to the PM.
8. Tidying the register of the site·app's Korean strings, replacing the store screenshots with real screens, and the iOS home name `Equipe`→`equipe` are proposed as **separate tickets**.

## Component · document focus view

- A long Markdown document may move from its right panel into the full app content area. It is the same document surface and scroll handle, not a second viewer or macOS full screen.
- The focus header is return-first: origin-specific back action plus `Esc` on the left, document tools on the right, then full filename and path on a second row. Never truncate the filename; let the header grow.
- At 760pt or 150% text, keep back and More document actions visible and move secondary tools into More. The reading measure stays `layout.measure`; diagrams reflow vertically before they overflow horizontally.
- Exiting by Back, `Esc`, or `⇧⌘F` restores the exact origin, split widths, selected ticket/document, scroll position, and keyboard focus. `Esc` closes an open menu before it exits focus view.
- Full interaction, copy, accessibility and QA contract: `design/specs/document-focus/spec.md` (T-617).

## Component · renaming a conversation

- One editor, wherever rename starts: a small card hanging 6pt under the header title (its left edge on the title, at most 360 wide, never outside the conversation pane), the app's popover look. Entries: the header title itself (a button: hover wash + pencil, tooltip Rename with the full title when cut), the first item of `This conversation` ⋯, and the sidebar row's pencil (beside ×, 24pt each), which opens the conversation and the same card.
- The card: label, one-line field with the whole title selected, a counter inside the field from 30 of 40 characters, a message line only when there is something to say, Cancel `Esc` · Save `↩`. Nothing is ever cut: past 40 the overflow is marked and Save waits. Return saves, Esc reverts, an outside click acts as Return; Korean composition takes Return and Esc first.
- Saving shows the new title in the header and sidebar at once; a failed save puts the old title back and reopens the card with what was typed and Try again.
- In a narrow bar the title has the room first: the context figure goes, then the project folds to its mark, then the project and its slash go; ⋯ always stays.
- Full interaction, copy (8 languages), accessibility and QA contract: `design/specs/rename-conversation/spec.md` (T-642).
