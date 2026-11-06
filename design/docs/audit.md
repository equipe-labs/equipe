# Audit — values scattered across 4 codebases (2026-10-16, designer)

Method: read each repository's token·theme files and counted literal uses with `grep` (a regex count — for spotting tendencies, may include comments·tests). The colour picture carries on from T-144's `audit.md` §2. **Unconfirmed** marks code that couldn't be read in full.
Sources: `equipe-site/src/styles/global.css`·`design/tokens.md` · `equipe/crates/desktop/src/theme.rs`·`motion.rs` · `equipe-app/design/tokens.json`·`android/features/.../Theme.kt`·`ios/Packages/Sources/**.swift`.

## 1. At a glance

| Axis | Site | Desktop | iOS | Android | v1 decision |
|---|---|---|---|---|---|
| Colour | 8 variables (light·dark) | palette of 24 fields + 8 member colours | `tokens.json` 12 colours×2 — code mostly uses system colours (`.secondary`, `systemBackground`) (**C6 unconfirmed**) | `Theme.kt` M3 scheme 14 colours×2 | 32 semantic colour tokens (§`tokens/semantic.color.*.json`) |
| Spacing | `--s-1..9`: 4·8·12·16·24·32·48·72·112 | 4·8·12·16·24 (+`S3`·`S6`) | 4·8·12·16·20·24·32 + 2·3·6·14 (literals) | `dp` 4·8·12·16·24·32 + 6·10·20·48·56·64 | the site's scale adopted (`space.1..9`) |
| Corners | 8 · 14 (+6·7·12·34 literals, `50%`) | 6·8·12·16 | 8·12·16·18 | 8·12·16 (+2) | 6·8·12·16·24·full |
| Type | 12·14·15·16·18·22·32·36·56 | 15/25·14/20·13/20·12/18 | system styles (footnote 11, subheadline 10, caption 6…) | M3 slots (bodyMedium 19, labelMedium 12, bodyLarge 9…) | 10 roles × per-platform values |
| Motion | 150ms in 3 places, `prefers-reduced-motion` in 2 | 120·160·200·180ms, cubic ease-out, a `reduce_motion` check | `.easeOut(0.15)` in 1 place, `accessibilityReduceMotion` in 1 | `tween` **0 hits** in a code search (M3 defaults) | 120·160·200·(180)ms, `out-cubic` |
| Shadow | 2 kinds (card `0 24 60 .14`, banner `0 4 16 .15`) | popover, 2 layers (`motion.rs`) | none | none (M3 default) | 2 kinds, `float`·`overlay` |
| Touch | 44px | 28 controls·36 fields·32 rows | 44 (`minTouchTarget`) | 48·56 | per-platform `size.*` |

## 2. What overlaps — merged

| The same thing | Current values | Merged value | Why |
|---|---|---|---|
| Body text (light) | `#1A1A17` · `#15161A` · `#0A0A0C` | `#15161A` | T-144 guide §6, contrast 18:1 |
| Dark background | `#121211` · `#0A0A0C` | `#0A0A0C` (void) | matches the app·desktop·icons |
| Accent (links) | `#1D4ED8` · `#2E5C8A` · none | `#1D4ED8` / dark `#8FB0FF` | T-144 decision D2 A |
| Danger (err) | desktop `#AD2E27`·`danger()` `#B3261E`, app `#A32A24` | `#AD2E27` / `#EE7B74` | one meaning had three values |
| Secondary text | `#57544C` · `#5E6068` · `#3F4148`/`#5B5E66` | `#4F5159` / `#A3A5AB` | the most neutral of those at 4.5:1 or more |
| Card corners | site 14 · desktop 12 · app 16 | web·desktop 12, mobile 16 | keeps platform conventions (larger corners on mobile) but *one name* (`radius.card`) |
| Control corners | 8 · 8 · 12 | web·desktop 8, mobile 12 | as above |
| Motion durations | 150 · (120/160/200) · 150/250 | 120·160·200 | the desktop's values were checked most finely |
| Icons | desktop 14·16 | 16·20·24 | 14 merged into 16 |
| Touch areas | 44 · 28 · 44 · 48 | per platform | pointer apps may use 28 |

## 3. What goes · what's newly needed

- **Goes**: the hard-coded colours of the site's `.phone`·`.pc`·`.pb` (`#1a1a17`, `#0a0a0c`, `#efede7`, `#b4cde6`, `#9a9ca3`…, `global.css:118-124`) — they're a *picture* inside the phone mock-up, so they belong in an SVG/image, not tokens. `equipe-app/design/tokens.json` is replaced by the v1 generated file (it was a document no code read — C6 unconfirmed). The colour·spacing tables in the site's `design/tokens.md` are replaced by `tokens/`.
- **Newly needed**: ① the semantic colour `ok` (done) — defined nowhere before ② status colour tints (`*-bg`) ③ `border-strong` (3:1 for field·secondary button borders — there was only the hairline) ④ `inverse` (toasts·tooltips) ⑤ `scrim` ⑥ `elevation.overlay` (sheets·dialogs) ⑦ one reduce-motion rule (`motion.*.reduced`) ⑧ `action-hover/pressed` (the desktop's `primary_hover()` (`theme.rs:300`)·`primary_press()` (L305) family as semantic tokens).
- **Not touched in v1**: the 8 member avatar colours (`theme.rs members`, a separate system for telling members apart, contrast not computed — as in T-144); the desktop sidebar's 3 colours (`side*`)·`prose`·`text2`·tooltip colours stay desktop-local values (no counterpart on other platforms) — tokens for them in v1.x if needed.

## 4. Not confirmed this time
- Whether iOS's `tokens.json` is actually wired into code (C6) — no `Color(…tokens…)` use was found in the SwiftUI code. Android *copies* the values into `Theme.kt` (only a comment refers to `tokens.json`).
- The site's `:lang()` type rules (`global.css:197-204`) are outside the v1 tokens' scope (font fallbacks are T-144 §3.3).
- The new values weren't rendered directly on the desktop's GPUI screens — only web sheet renders. Every screen change goes through design QA (a squad decision).
