# equipe design system

The site, desktop, iOS and Android use values from **one source** (`tokens/*.json`, DTCG). This folder is that source, and the equipe desktop's **Design system** screen reads its `tokens.json`·`guidelines.md`·`assets/`·`components/` (`docs/DESIGN-SYSTEM.md`). The foundation is the T-144 brand guidelines v1, the tokens are the T-145 design system v1, and the folder layout is T-149.

## Generated files are never edited by hand

| Source edited by hand | Generated files (made by `npm run build` and the like — **don't touch**) |
|---|---|
| `tokens/primitive.json` → `semantic.json`·`semantic.color.light.json`·`semantic.color.dark.json` → `component.json` | `tokens.json` (the merged file the screen reads, light·dark) |
| | `build/css/tokens.css`·`compat-site.css` · `build/rust/tokens.rs` · `build/swift/EquipeTokens.swift` · `build/kotlin/EquipeTokens.kt` |
| the list of pairs in `scripts/contrast.mjs` | `docs/contrast.md` |
| `scripts/sheets.mjs`·`reference/components.css` | `components/*-{light,dark}.png` (`npm run sheets`) |

To change a value or a name, edit `tokens/` and run `npm run check`. If the generated files' format (names·units) is the problem, edit `scripts/build.mjs`. Edit only a generated file and it vanishes at the next build, leaving a value on screen that disagrees with the source. Other repositories (equipe-site·equipe-app) copy the generated files and don't edit them there either. Each copy is listed in `copies.json`; `npm run check:copies` here, and one line per copy in each repository's CI (`curl … | diff`), catch a copy out of step (`docs/DESIGN-SYSTEM.md`, 'Copies').

## Folder

```
design/
  tokens.json        generated — the DTCG file the desktop's Design system screen reads
  guidelines.md      the rules in sentences (T-144 brand + principles) — the screen's 'Guidelines'
  CHANGELOG.md       token·component·brand changes, one line each
  tokens/            the source, three layers (primitive / semantic(.light|.dark) / component)
  build/             generated — code per platform (css·rust·swift·kotlin); build/sheets/ holds sheet HTML (not committed)
  components/        component sheet PNGs (light·dark) + the rules in components.md
  assets/logo/       mark.svg, wordmark-light.svg, wordmark-on-dark.svg
  brand/             brand groundwork: brand.md (strategy·voice), audit.md, contrast.md, comparison·logo-usage sheets
  docs/              audit.md (T-145 audit), adoption.md (platform adoption plan), contrast.md (generated)
  reference/         web reference implementation components.css
  scripts/           build · contrast · lint-css · sheets
```

## Regenerating
```sh
cd design
npm ci               # style-dictionary ^5, Node 22+ (used to resolve references; output comes from the formatters in scripts/build.mjs)
npm run build        # tokens/ → tokens.json + build/{css,rust,swift,kotlin}
npm run contrast     # contrast check → docs/contrast.md (exit code 1 if any pair falls short)
npm run lint:css     # no raw values in reference/components.css
npm run check        # the three above at once
npm run sheets       # re-render components/*.png (needs Chrome; fonts over the network from Google Fonts·jsDelivr)
npm run check:copies # the other repositories' copies listed in copies.json match the generated files (../<repo> or <repo>=<path>)
```
The tests in `crates/core` (`design::tokens`) read `tokens.json` with the daemon's own parser and check it has no problems and has both light and dark.

### How `tokens.json` is made
- The four source files are merged on the light values, and `{references}` are kept as they are (the screen shows the alias and resolves it itself).
- Only tokens whose value differs in the dark file get the dark value in `$extensions.mode.dark`. `$value` is light.
- Platform differences stay in `$extensions["im.equipe"].platform`, and `$value` is the default (web) value — the screen shows the default.
- If light and dark have different keys, the build fails.

### Platform output rules
| | Colour | Length | Time | Type | Mode |
|---|---|---|---|---|---|
| CSS | `#RRGGBB(AA)`, `--eq-*` | px | ms | `font` shorthand + `-tracking` | `:root` (light) · `prefers-color-scheme`+`[data-theme]` (dark) · narrow screens·reduce motion via `@media` |
| Rust | `0xRRGGBBAA` `u32`, `Palette` struct `LIGHT`/`DARK` | `f32` logical px | `u64` ms | `TextStyle` | `palette(dark)` |
| Swift | `Color(0x…)`, `EquipeColors.light/.dark` | `CGFloat` pt | `TimeInterval` seconds | `EquipeTextStyle` (Dynamic Type) | `.resolved(colorScheme)` |
| Kotlin | `Color(0xAARRGGBB)`, `EquipeColors.Light/.Dark` | `Dp`·`sp` | `Int` ms | `EquipeTextStyle`·`Typography` | `equipeMaterialColors(dark)` |

The desktop takes spacing·radius·size·type·motion from `build/rust/tokens.rs` (T-160). Its colours equal the tokens since 2.0.0 (T-699, `docs/t699-palette-map.md`) but `theme.rs` still writes them out; wiring `Palette` to `tokens::palette()` is a follow-up. The site, iOS and Android don't use the generated files yet — the order of adoption is in `docs/adoption.md`.

## Governance

**Version** — `version` in `package.json` and `CHANGELOG.md` carry the same number.
- **Patch (1.0.x)**: fine value adjustments, descriptions·docs, changes that *raise* contrast.
- **Minor (1.x.0)**: *adding* tokens·components·states. Existing names stay.
- **Major (x.0.0)**: renames·removals, visible value changes. A retired token is marked `$extensions["im.equipe"].deprecated` for one minor and removed at the next major.

**Changing a token**
1. Write it on a ticket: what·why·"why no existing token will do" (required for a new value). Semantic tokens are named for their use (`color.ok`, not `color.green`).
2. Edit `tokens/*.json` and run `npm run check` (it stops here if contrast breaks). Light and dark both.
3. Re-render the sheets with `npm run sheets` and compare before/after.
4. One line in `CHANGELOG.md` (version·date·why·platforms affected).
5. Make a code ticket per platform to bring the generated files into each repository·screen. **If a screen changes, designer QA is required.**

## Open · unconfirmed
1. `assets/logo/*.svg` are **text-based SVGs** drawn to the T-144 §1 spec (Helvetica Neue Bold, dot `#3F6FF5`) — on a device without the font they show in Arial. The outlined master waits on confirming Helvetica Neue's redistribution licence (T-144 §7-1).
2. Everything else unconfirmed is as in T-145: iOS·Android default text sizes not rechecked, the newly chosen colours (`elevation.overlay`·`inverse`·`*-bg` tints·`ok`) not yet checked against taste on screen, the Pretendard·Inter licence texts not read, the 8 member avatar colours' contrast not computed.
