# T-144 brand audit — equipe as it looks now (2026-10-16, designer)

Direction (the person's decision, 2026-10-16): **keep the name·logo; tidy up and systematise colour·type·voice·guidelines.** This document only records "what is in use now and where it disagrees". The prescriptions are in `brand.md` (strategy)·`guidelines.md` (rules).
At a glance: **`audit-board.png`**. Screenshot originals: `audit-shots/` (the site light·dark, live captures 2026-10-16). Each value's source is given as a file path. What couldn't be seen directly is **unconfirmed**.

## 0. Conclusion in a line

equipe has **three faces** today. ① The site = blue 2×2 dots + blue links, warm paper / ② desktop·app icon·store = a black tile `e.` + a grey dot, steel blue·black, Helvetica / ③ the iOS·Android app screens = system colours, no accent colour. Logo·black·accent·typeface·Korean register·supported languages all differ from place to place. The biggest rifts are the **logo** (the site vs everything else) and the **accent colour** (three kinds).

## 1. Logo·mark·name

| # | Where | Now | Source |
|---|---|---|---|
| L1 | Site header·404·JSON-LD logo | blue (`#1D4ED8`) 2×2 dots, one filled + three translucent. Wordmark `equipe` (no dot) | `equipe-site/src/components/Mark.astro`, `public/logo.png` / live capture `audit-shots/site-light.png` |
| L2 | Site favicon·home icon | favicon = black tile + white `e` (T-105's record), `public/app-icon.svg`·`design/mark.svg` = blue tile + four white dots → **even within the site, favicon and mark differ** | `equipe-site/public/favicon.svg`, `design/mark.svg`, T-105 `brand.md` |
| L3 | Site OG | blue four-dot tile + wordmark `equipe` | `public/og.png` (`audit-board.png` ③) |
| L4 | Mac app icon (T-88) | void (`#0A0A0C`) tile, bone `e`, **grey (`#8A909B`) square full stop** | `outputs/T-88 …/png/AppIcon-mac-*.png`, `src/icon-mac.html:14` |
| L5 | iOS·Android app icon (T-72) | the same family as L4 | `outputs/T-72 …/png/ios`. Android's current `ic_launcher.xml` carries a "Placeholder" comment (T-105's record, not rechecked this time) |
| L6 | Store feature graphic | wordmark `equipe.` (with the dot) + a grey **square** dot | `outputs/T-72 …/png/play/feature-graphic-ko-1024x500.png` |
| L7 | GitHub organisation avatar | white background + black `e` + a chrome-grey dot ("label") | `equipe/docs/BRAND.md` Assets |
| L8 | How the name is written | site `equipe` / BRAND.md·Android `wordmark` `equipe.` / iOS home-screen name: `project.yml:61` says `Equipe` (capitalised), `:96` of the same file `equipe` — **a conflict inside one file** / Android `app_name` `equipe` / the glossary says "always lowercase `equipe`" | each file, `equipe-site/design/i18n/glossary.md` §1 |
| L9 | Shape of the full stop | app·store: the font's (Helvetica) **square** full stop / T-105 option C: **circle** / the site's four dots: circles | `icon-mac.html`, T-105 `brand.md` |

→ Prescription: settle one logo master (`guidelines.md` §1, needs the person's decision), one rule for writing the name (§1.2).

## 2. Colour

Site values from `equipe-site/design/tokens.md`·`src/styles/global.css`, desktop from `crates/desktop/src/theme.rs:113-163`, app from `equipe-app/design/tokens.json`.

| Role | Site light/dark | Desktop light/dark | App tokens (iOS·Android) light/dark |
|---|---|---|---|
| Background | `#FAF8F4` / `#121211` | `#F6F5F1` / `#0A0A0C` | `#F6F5F1` / `#0A0A0C` |
| Card | `#F0EDE6` / `#1B1B19` | `#FFFFFF` (raised) / `#141417` | `#FFFFFF` / `#16171A` |
| Body text | `#1A1A17` / `#F2EFE8` | `#15161A` / `#EFEDE7` | `#0A0A0C` / `#EFEDE7` |
| Secondary text | `#57544C` / `#A9A59B` | `#5E6068` / `#8A8C93` | `#3F4148`·`#5B5E66` / `#A3A5AB`·`#7C7E85` |
| **Accent (links·buttons)** | **`#1D4ED8` / `#8FB0FF`** | **`#2E5C8A` / `#B4CDE6`** | **none** — accent = text colour (`#0A0A0C` / `#EFEDE7`) |
| Filled button | blue fill + white text | near-black `#15161A` / steel `#2F4256` | text-coloured fill |
| Danger | no site UI | `#AD2E27` / `#EE7B74` | `#A32A24` / `#E8837D` |
| In progress·waiting | — | amber `#825600`/`#E2B865`, orange `#A04610`/`#EC9A62` | not in the app tokens (undefined) |
| Focus ring | `--accent` | — | `#2F5E8C` / `#B4CDE6` |

Disagreements:
- **C1 Two blacks**: the site's dark `#121211` (warm) vs app·desktop `#0A0A0C` (void). The light backgrounds too: `#FAF8F4` vs `#F6F5F1`.
- **C2 Three accents**: bright blue `#1D4ED8` / steel `#2E5C8A` / none. The same "link" is a different blue, or not blue, from place to place.
- **C3 Conflict with `BRAND.md`**: BRAND.md says "no second accent colour, monochrome + chrome", while the site has a blue accent across its whole UI (`equipe/docs/BRAND.md` Palette).
- **C4 Semantic colours only on the desktop**: amber (in progress)·orange (your turn)·red (error) exist only in `theme.rs`, not in the app tokens·site. A done (green) semantic colour is **defined nowhere** (the greens in `theme.rs`, like `0x28803C`, are among the 8 member avatar colours).
- **C5 The 8 member avatar colours** (`theme.rs:135, 163`) are a system apart from the brand palette. Contrast not recomputed this time (**unconfirmed**).
- **C6 Some app tokens aren't used in code**: iOS `RootView.swift` uses `Color(.systemBackground)` — whether `tokens.json` is wired in couldn't be traced all the way this time (**unconfirmed**). In Android's `themes.xml` only `#FFF6F5F1` was found.
- Contrast: the site's values are documented as AA-checked (`tokens.md`), and the desktop's `theme.rs` comments state a 4.5:1 intent. This time **only the three new palette options were computed** (`contrast.md`). The current values weren't re-measured.

## 3. Typefaces

| Where | Now | Source |
|---|---|---|
| Site | Inter Variable (self-hosted), mono JetBrains Mono Variable, ko falls back to `Apple SD Gothic Neo`/`Noto Sans KR`/`Malgun Gothic` | `global.css:10-11,197`, `package.json` (`@fontsource-variable/*`) |
| Desktop | `SANS = "Helvetica Neue"`, `MONO = "Menlo"`; body 15/25, UI 14, small 13, meta 12 | `theme.rs:190,236` |
| iOS | system (SF Pro), presumably | no font set in a code search (T4) |
| Android | Material default (Roboto), presumably | no font set in a code search (T4) |
| Wordmark·icons | Helvetica Neue Bold, tracking −6px@150px (BRAND.md) / −0.06em (icon HTML) | `BRAND.md`, `icon-mac.html:14` |
| Store feature graphic | Helvetica family + the system font for Hangul | `feature-graphic-ko` |

- **T1** The site (Inter) and the desktop·logo (Helvetica Neue) differ, so the logo's letters are in a different typeface from the site's body. Helvetica Neue is Apple-only, absent on the web·Android.
- **T2** Two tracking values (about −0.04em / −0.06em).
- **T3** The site CSS already defines ko·ja·zh-CN fallbacks (`global.css:197-204`: Apple SD Gothic Neo / Hiragino / PingFang SC and others + `word-break: keep-all`, `line-break: strict`). **Inter covers es·pt-BR·de·fr as is** — no problem. What's missing is zh-Hant (not on the site) and a document of fallback rules for the apps (iOS·Android).
- **T4** No font set separately in iOS (`.custom(`·`UIFont(name`)·Android (`fontFamily`) code came up in a search → both appear to use system fonts (not the whole code read).

## 4. Icons·illustration·screenshots

- Desktop: **Lucide** icons bundled via GPUI Kit (`icons.rs:1`), two sizes (`ICON`, `ICON_SM`), following the text colour.
- iOS: SF Symbols (`systemName` in 21 places — `ios/Packages/Sources`). Android: depends on Material3 (the icon set is **unconfirmed**).
- Site: the home illustration is inline SVG built on blue dots·lines (`.d-frame`, `.d-fill` classes — `global.css:100-102`), one real desktop screen PNG (`src/assets/desktop-tickets.png`) + a phone mock-up captioned "coming" (`.phone`, `global.css:121`).
- Store screenshots (T-72): currently **placeholder frames** (no real app screens, `audit-board.png` ⑧). Captions in Korean's polite -yo register, with English capitals copy like `APPROVE`·`EQUIPE` on top.
- BRAND.md's motifs ("fashion house for software teams", chrome objects·film grain·gallery placards) are **used on no product screen** (only the demo social cards·avatars).
- **I1** No documented rule for icon stroke·corners. **I2** No illustration rules. **I3** No screenshot frame rules (device frame·background colour·caption position live only in T-72's `README.md`).

## 5. Voice·copy

**English (source: `equipe-site/design/copy.md`)**: "plain, concrete, a little dry. No hype words". The name always lowercase. The live site broadly follows this rule.

**Disagreements**
- **V1 Two main lines**: `copy.md` H1 = "Your AI software squad, led by you." / live site H1 = "Lead a team of AI agents. You approve every merge." / `llms.txt` = "an open-source AI software squad…". There's no canonical one.
- **V2 Use of 'squad'**: the ko meta title "an AI coding agent squad led by one person", the store "AI squad remote"; the live hero doesn't use 'squad' (nor does the English hero). The term carries different weight from place to place.
- **V3 Korean register** — sentence endings counted with a simple regular expression (approximate; noun forms·fragments excluded):

| Where | Strings | formal endings (-mnida/-seumnida) | polite -yo endings |
|---|---|---|---|
| Site `ko.ts` | 143 (6+ characters) | 13 | **79** |
| Store copy (listing.md) | — | almost none | -yo register (the document says "matching the site") |
| Desktop `ko.json` | 1330 | **235** | 182 |
| iOS `Localizable.xcstrings` | 202 | **45** | 22 |
| Android `values-ko` | 197 | **43** | 25 |

  The site·store use the -yo register, the app screens lean formal, and even one Android screen mixes them (e.g. a polite "scan" next to a formal "doesn't", `values-ko/strings.xml`). The counting method is in this document's appendix.
- **V4 Second person**: the Korean site uses "you" (dangsin) (e.g. "without your approval") — app strings mostly drop the subject. English uses "you/your".
- **V5 English capitals copy**: store screenshots' `APPROVE`, `EQUIPE`, and symbol·capital decoration in a few places like the desktop's "⚑ permission request" (BRAND.md's "tracked caps" is visual language only, not a copy rule).

## 6. Supported languages

- **Site, 8**: en, ko, ja, zh-CN, es, pt-BR, de, fr (`src/i18n/locales.ts`).
- **Desktop, 8**: de, en, es, fr, ja, ko, zh-Hans, zh-Hant (`crates/desktop/locales/`).
- → **pt-BR only on the site, zh-Hant only on the desktop.** iOS·Android languages: only iOS `Localizable.xcstrings`·Android `values-ko` checked (en·ko). The rest **unconfirmed**.
- This ticket's "8-language sample" was rendered as the site's 8 + one zh-Hant line (for the desktop) (`type-8lang.png`).

## 7. Priorities (proposed)

| Rank | Item | Why |
|---|---|---|
| 1 | L1–L9: decide the logo master and the dot (colour·shape) | the first impression the person sees every time. Before the app ships, changing it costs least (T-105) |
| 2 | C2: one accent colour, C1: tidy the blacks | the precondition for tokens (the next ticket) |
| 3 | T1: typeface rules (web Inter, apps system, logo an outlined SVG) | so the logo isn't tied to a font |
| 4 | V1·V3: canonical copy + one register per language | translations·store copy in the same voice |
| 5 | Language lists aligned (pt-BR·zh-Hant) | a product decision — outside design's scope, handed to the PM |

## Appendix — counting method and limits

- Korean register: strings taken from `locales/ko.json`·`Localizable.xcstrings`·`values-ko/strings.xml`·`i18n/ko.ts`, counted by regular expression for whether they end in the formal `-nida` or the polite `-yo`. Strings with only numbers·single words·variables are neither, so the totals are below the string counts. For spotting the tendency, not a translation review.
- Live site captures: 1280×800, the English pages. No Korean·mobile-width captures.
- Desktop·app UI was checked from code·the repositories' PNGs, not by launching the real apps.
