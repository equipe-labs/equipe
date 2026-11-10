# Adoption plan per platform (for frontend·mobile code tickets)

Principle: **one platform = one ticket**, in the order ① settle where the token source lives (the PM's·the person's decision, §0) → ② desktop (values change least) → ③ site → ④ Android → ⑤ iOS (last, since it's in release review). Every screen change **requires designer QA** (squad decision 2026-10-16) — each ticket's acceptance criteria include "light·dark before/after screenshots + designer approval". Files·line numbers are as of 2026-10-16 (`main`) — check again when starting.

## 0. To decide first (before the code tickets)

| # | Decision | Proposal | Why |
|---|---|---|---|
| A | **Which repository holds the token source (`tokens/*.json` + `scripts/`)** | keep it in the `equipe` repository's `design/system/`; the site·apps **commit the generated files** (each in `design/generated/`). In CI, `npm run check && git diff --exit-code` | `equipe` already has `docs/BRAND.md`, and the desktop lives in it. Committing generated files means the site·app builds need no Node. The person decided "assets per project", so one source, a copy each |
| B | **Settle the product UI's primary button = a text-coloured fill** (black/bone) | exactly T-144 §2.2's proposal | the desktop's dark primary button changes a lot, steel `#2F4256` → bone (§2 risks) |
| C | Pin the Style Dictionary version in the code tickets (`^4`) | `package.json` as it is now | the formatters are custom and may break on a major upgrade |

## 1. Shared: using the generated files
`npm install && npm run check` → `build/` is made again from the same source. Engineers **never edit generated files by hand**. When a value is needed, follow the change procedure in `governance` (README §Governance). Verified: Rust compiles with `rustc --crate-type lib` ✅ · Swift `swiftc -typecheck` (iOS simulator SDK) ✅ · Kotlin: `equipe-app/android` copied to `/tmp` passes `:features:compileDebugKotlin` ✅ (including explicit API mode) · CSS rendered as sheets in Chrome ✅. **Not verified**: actually wiring it into GPUI and rendering, actually applying it to SwiftUI·Compose screens.

## 2. Desktop — `equipe/crates/desktop`

**Files to change**
1. `design/system/…` → copy to `crates/desktop/src/tokens.rs` (= `build/rust/tokens.rs`), `mod tokens;` in `main.rs`.
2. `src/theme.rs` `Palette` (`dark()` L111, `light()` L139): fill the fields from `tokens::palette(dark)` by the mapping below (used directly via `rgba(u32)`).

| `Palette` | Token | Value change |
|---|---|---|
| `bg` `text` `raised` (→`color_raised`) `code_bg` (→`color_sunken`) | `bg` `text` `raised` `sunken` | light `code_bg` `#ECEBE6`→`#EAE7E0`, dark `raised` `#141417`→`#1B1C20` (unified with the app) |
| `muted` | `muted` | light `#5E6068`→`#4F5159`, dark `#8A8C93`→`#A3A5AB` |
| `accent` `accent_bg` `on_accent` | `accent` `accent-bg` `on-accent` | **`#2E5C8A`→`#1D4ED8`, dark `#B4CDE6`→`#8FB0FF`** (link·selection colours change visibly) |
| `primary` `on_primary` + `primary_hover()` `primary_press()` (L300~) | `action` `on-action` `action-hover` `action-pressed` | light nearly the same (`#15161A`). **Dark `#2F4256` (steel) → `#EFEDE7` (bone)** — a big change, decision B |
| `amber` `orange` `red` | `work` `wait` `err` | same values |
| `danger()` `danger_bg()` (L289~) | `err` `err-bg` | `#B3261E`/`#F0918A` → `#AD2E27`/`#EE7B74` (one danger value) |
| `hair` | `line` | alpha `0x24`, the same family |
| stays local | `side` `side_hover` `side_sel` `wash` `prose` `text2` `members` `popover` `tip_*` | no tokens — as they are |

3. The constants in `theme.rs` `S3,S6,R_SM..R_XL,CONTROL,FIELD,ROW,ICON,ICON_SM,GAP,BODY/UI/SMALL/META(+_LINE)` — **the values are the same** (`SPACE_*`, `RADIUS_*`, `SIZE_BUTTON`, `SIZE_FIELD`, `SIZE_ROW`, `TYPE_*`). Replace each with a one-line re-export from `tokens`. `ICON_SM` 14 → 16 (`SIZE_ICON_SM`) affects the screen.
4. `src/motion.rs` `FAST/BASE/SLOW/SLIDE` = `MOTION_DURATION_*_MS` (same values), `POP_TRAVEL/RISE_TRAVEL/DIALOG_TRAVEL` = `MOTION_TRAVEL_*`, `float_shadow()` = `ELEVATION_FLOAT` (same values), and the `reduce_motion` branch goes to `MOTION_DURATION_REDUCED_MS` (a 100ms fade) — today it's "the end state at once", so an *optional* change.
5. Components: the decision card (`decide.rs`)·toast·sheet (`sheet.rs`) onto the `decision.*`·`toast.*`·`sheet.*` tokens — a separate follow-up.

**Risks**: ① the dark primary button's colour change (decision B) ② the link·selection blue changes ③ `ICON_SM` 14→16 changes sidebar spacing ④ the highlight `accent_bg` doubles as the selected row colour and may clash with `side_sel` ⑤ member and project colours untouched. **Size**: medium (colour mapping + screenshot comparison, light·dark × the 8 main screens). **QA**: before/after PNGs of the same screens.

## 3. Site — `equipe-site`

**Files to change**
1. Copy `src/styles/tokens.css` (= `build/css/tokens.css`) + `compat-site.css`; import both from `src/layouts/*` (or the top of `global.css`).
2. `src/styles/global.css` `:root` (L2-L22): **delete** the colour·spacing·radius·`--tap` declarations and let the aliases in `compat-site.css` stand in (the rules keep using `var(--bg)` and the like → nothing breaks at once). Then move the rules to `--eq-*` step by step, and delete `compat-site.css` when done.
3. Hard-coded values: `font-size` literals 14/15/16/17/18/19/22 (L35-L67, L114-L178) → `font: var(--eq-type-*)` (the `:lang()` rules stay), `border-radius: 34px/12px/6px/7px` (L118-L124) and the `.phone` colours are §audit 3 — move them into an image or leave them with a TODO.
4. Component CSS starts from `reference/components.css` (`.btn`→`.eq-btn--brand`, `.card`, `.chip`, `details`→row, `.lang-banner`→a banner instead of a toast).
5. `design/tokens.md` becomes one line linking `design/system`; `Mark.astro` is outside this ticket (the T-144 logo ticket).

**Value changes**: `--bg` `#FAF8F4`→`#F6F5F1`, dark `#121211`→`#0A0A0C`; `--surface` (cards) `#F0EDE6`→`#FFFFFF` (**in light, cards become lighter than the page** → cards gain a hairline); `--r-2` 14→12; `--text`/`--muted` change slightly; `--accent`·`--on-accent` the same. All recorded in comments in `compat-site.css`.
**Risks**: ① card fills·corners change the visual weight of the home layout ② photo·screenshot edges on dark `#0A0A0C` ③ all 8 translated pages need checking ④ Astro builds without CSS variable name clashes (low, thanks to the `--eq-` prefix) ⑤ caching: check the static CSS hashed filenames. **Size**: medium-large (small if it starts with aliases). **QA**: live 8 languages × light·dark × 1280/390 widths.

## 4. Android — `equipe-app/android`

**Files to change**
1. Copy `features/src/main/kotlin/im/equipe/design/EquipeTokens.kt` (= `build/kotlin/EquipeTokens.kt`, **explicit API mode handled·compile checked**). Splitting it into a separate `:design` module lets `features`·`app` share it (recommended — optional).
2. `features/…/Theme.kt` `EquipeTheme`: `MaterialTheme(colorScheme = equipeMaterialColors(dark), typography = EquipeTypography)` + `CompositionLocalProvider(LocalEquipeColors provides …)`. Delete the existing `Dark`/`Light` schemes (14 colours) — the current mapping (surface=bg, surfaceVariant=card …) is carried over.
3. Literals in screen code: `16.dp` (31 times) `12.dp` (21) `56.dp` (19) `8.dp` (15) `48.dp` (7) `24.dp` (6) → `EquipeSpace.s4` and so on, `RoundedCornerShape(12/16/8.dp)` → `RoundedCornerShape(EquipeRadius.control/card…)`. Call sites via `grep -rn "\.dp" android --include=*.kt`.
4. The window background in `app/src/main/res/values{,-night}/themes.xml` (`#FFF6F5F1` and others) → the `bg` value. Android icons·splash belong to the logo ticket.
5. Components: the decision card·sheet·toast as new `@Composable`s (`EquipeButton`, `EquipeBadge` …) gathered in `features/…/ui/` — the sheet PNGs are the reference.

**Value changes**: the app has **no accent colour** today (`primary` = the text colour). In v1 the primary button is still a text-coloured fill (decision B), so buttons stay, and blue (`accent`) *newly* appears on links·selection·focus. `outline`/`secondaryContainer` become `line`/`sunken` (dark `#26272C`→`#111114`). Typography: M3 defaults → the role mapping (`bodySmall` 12→13 sp, `labelMedium` stays 12).
**Risks**: ① check every place Material components (`OutlinedTextField`, `NavigationBar`) use scheme colours (**unconfirmed** — the whole code wasn't read) ② at the largest `sp` scale (font size setting at max), two lines overflowing a 56dp `row` ③ keep dynamic colour switched off ④ Macrobenchmark cold start: `EquipeColors` is an `object`/top-level `val`, so initialisation cost is tiny, but it needs measuring per CLAUDE.md's requirement. **Size**: medium. **QA**: emulator (android-35) light·dark × font size default/max.

## 5. iOS — `equipe-app/ios`

**Files to change**
1. Copy `Packages/Sources/Features/EquipeTokens.swift` (= `build/swift/EquipeTokens.swift`, `swiftc -typecheck` checked — the `Color(0x…)` helper is a `private extension`, so no clash) or split it into a new `Packages/Sources/Design` package (possible, since the API is `public`).
2. Add an `EquipeThemeModifier` (one hand-written file, ~15 lines) that injects `@Environment(\.colorScheme)` → `EquipeColors.resolved(scheme)` at the root — the generator doesn't make SwiftUI Environment keys.
3. `Packages/Sources/Features/*.swift`: `.foregroundStyle(.secondary)` → `colors.muted`, `Color(.systemBackground)` (`RootView.swift`) → `colors.bg`, literals like `.padding(16)`·`spacing: 8` (most common: `spacing: 0`/14, `spacing: 8`/11, `.padding(.horizontal, 16)`/7, `.padding(32)`/6) → `EquipeSpace.*`, `cornerRadius: 12/16/18` (5 places) → `EquipeRadius.control/card` (18 tidied to 16); `.font(.footnote)` and the like **stay** (already system Dynamic Type with the same roles: footnote=`small`, subheadline=`ui`, caption=`meta`, callout.monospaced=`code`).
4. Bring `ApprovalSheetView` (`DecisionViews.swift`) in line with the decision card's construction (badge·who·where·content·note·two buttons) — the structure is already similar. The `reduceMotion` branch goes through `EquipeMotion.animation(_, reduceMotion:)`.
5. `ios/design/tokens.json` can be deleted (no code reads it — after confirming C6). The AccentColor in `Assets.xcassets` becomes `accent` (a light/dark pair).
6. Icons stay SF Symbols.

**Value changes**: system colours → token colours (background `systemBackground` white/black → `#F6F5F1`/`#0A0A0C`, secondary text `.secondary` → `muted`). Blue `accent` new on links·`tint`.
**Risks**: ① **the first App Store review is submitted (T-117)** — put this in v1.1 *after* review·release, or, if during review, the person judges whether it carries no rejection risk (the change is visual only, not behavioural, but the screenshots change → the store screenshots need replacing) ② `.secondary` adapts automatically to vibrancy and the accessibility "Increase contrast" setting, token colours don't → add a rule raising it to `text` when `UIAccessibility.isDarkerSystemColorsEnabled` (proposed, not implemented) ③ iPad pointer hover: an overlay instead of SwiftUI's `.hoverEffect` (undecided) ④ at the largest Dynamic Type (AX5), no fixed 44pt `row` height (minimum height only) ⑤ the `LaunchTiming` 1-second cold-start requirement — static `let`s only, so little effect (needs measuring). **Size**: large. **QA**: simulator light·dark × 3 text sizes × the 6 main screens.

## 6. Split into tickets (proposed)

| Order | Title | Owner | Size | Depends on |
|---|---|---|---|---|
| 1 | Move the design system source into its repository + CI (`check`) | frontend | small | decision A |
| 2 | Desktop: bring in `tokens.rs` + replace the palette·constants | frontend (desktop) | medium | 1, decision B |
| 3 | Site: token CSS + aliases → move the rules | frontend | medium | 1 |
| 4 | Android: replace `EquipeTheme` + tidy dp·shape literals | mobile | medium | 1 |
| 5 | iOS: `EquipeTokens` + theme injection + tidy literals | mobile | large | 1, release schedule |
| 6 | New decision card·toast·sheet components (desktop·iOS·Android) | frontend·mobile | large | 2·4·5 |
| 7 | Design QA per platform (before/after, light·dark) | designer | small×4 | 2–6 |
