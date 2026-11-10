# T-699 — desktop `Palette` ↔ generated colour tokens

The desktop palette on `main` (`crates/desktop/src/theme.rs`, `Palette::dark()`/`light()` and its methods) is the
reference: it is what users see today. Tokens 2.0.0 take its values; nothing on the desktop changes. Wiring
`theme.rs` to `tokens::palette()` is a follow-up frontend ticket; until then this table is the contract, checked
value by value (light and dark) against `build/rust/tokens.rs` — 41 of 41 equal.

Alphas: gpui's `Rgba { a: 0.12 }` / `0.08` are written as the nearest 8-bit alpha (`1F` = 0.122, `14` = 0.078).

## Palette fields

| `Palette` | Token (`build/rust` field · CSS var) | Light | Dark |
|---|---|---|---|
| `bg` | `color.bg` | `#FAF9F6` | `#151513` |
| `side` | `color.side` · `sidebar.bg` | `#F3F2EF` | `#100F0E` |
| `side_hover` | `color.side-hover` · `sidebar.row-hover-bg` | `#EEEDE8` | `#1C1C1A` |
| `side_sel` | `color.side-selected` · `sidebar.row-selected-bg` | `#E7E6E1` | `#272624` |
| `wash` | `color.hover` | `#0000000C` | `#FFFFFF12` |
| `text` | `color.text` | `#181713` | `#F3F0E9` |
| `prose` | `color.prose` | `#24231F` | `#E5E2DB` |
| `text2` | `color.text-soft` | `#45443E` | `#CCC8C2` |
| `muted` | `color.muted` | `#62605A` | `#A2A09A` |
| `hair` | `color.line` | `#0000001A` | `#FFFFFF1A` |
| `accent` | `color.accent` | `#2160AB` | `#7EB8F3` |
| `accent_bg` | `color.accent-bg` | `#2160AB14` | `#7EB8F31F` |
| `code_bg` | `color.sunken` | `#F2F1ED` | `#1B1A19` |
| `raised` | `color.raised` | `#FFFFFF` | `#1F1E1C` |
| `primary` | `color.action` | `#181713` | `#ECEAE4` |
| `on_primary` | `color.on-action` | `#FAF9F6` | `#151513` |
| `on_accent` | `color.on-accent` | `#FAF9F6` | `#151513` |
| `amber` | `color.work` | `#865901` | `#EABA67` |
| `orange` | `color.wait` | `#B83A0B` | `#FF7A45` |
| `red` | `color.err` | `#AA2A27` | `#F2857D` |
| `members[0]` | `color.member-0` | `#835600` | `#E3B667` |
| `members[1]` | `color.member-1` | `#943C6B` | `#F4A0C8` |
| `members[2]` | `color.member-2` | `#0065A4` | `#7FC5FF` |
| `members[3]` | `color.member-3` | `#0D7435` | `#89D298` |
| `members[4]` | `color.member-4` | `#674EA1` | `#C3B0FD` |
| `members[5]` | `color.member-5` | `#006F68` | `#5AD5CB` |
| `members[6]` | `color.member-6` | `#984500` | `#F7A97C` |
| `members[7]` | `color.member-7` | `#5F6700` | `#BBC671` |

## Palette methods

| Method | Token | Light | Dark |
|---|---|---|---|
| `popover()` | `color.popover` · `popover.bg` | `#FFFFFF` | `#252422` |
| `popover_hover()` | `color.popover-hover` · `popover.row-hover-bg` | `#F2F1EE` | `#2F2E2B` |
| `tip_bg()` | `color.tooltip` · `tooltip.bg` | `#232220` | `#2F2E2C` |
| `tip_text()` | `color.on-tooltip` · `tooltip.fg` | `#F3F0E9` | `#F3F0E9` |
| `tip_muted()` | `color.on-tooltip-muted` · `tooltip.secondary-fg` | `#A9A7A1` | `#A9A7A1` |
| `danger()` | `color.err` (same as `red`) | `#AA2A27` | `#F2857D` |
| `danger_bg()` | `color.err-bg` | `#AA2A2714` | `#F2857D1F` |
| `primary_hover()` | `color.action-hover` | `#2C2B27` | `#FFFFFF` |
| `primary_press()` | `color.action-pressed` | `#050504` | `#D6D3CC` |
| `press()` | `color.pressed` (the wash × 2) | `#00000018` | `#FFFFFF24` |
| `switch_on()` | `color.switch-on` · `switch.track-on` | `#2160AB` | `#4378AD` |
| `switch_off()` | `color.switch-off` · `switch.track-off` | `#C9C7C2` | `#42413E` |
| `knob()` | `color.switch-knob` · `switch.knob` | `#FFFFFF` | `#F3F0E9` |
| `primary_key()` | stays a formula: `mix(on_primary, primary, 0.3)` of the two tokens above | — | — |

## Tokens with no `Palette` counterpart

Kept for the other platforms, re-based on the same warm neutral and hues:

| Token | Light | Dark | Why this value |
|---|---|---|---|
| `color.surface` | `#FFFFFF` | `#1F1E1C` | = `raised`: the desktop has no separate card fill |
| `color.focus` | `#2160AB` | `#7EB8F3` | = `accent` |
| `color.dot` | `#3F6FF5` | `#3F6FF5` | brand dot, unchanged |
| `color.border-strong` | `#86847E` | `#77756F` | new warm grey at ≥ 3:1 on every surface it meets (the desktop draws fields with `hair`) |
| `color.scrim` | `#00000066` | `#00000099` | black instead of the old cool ink |
| `color.inverse` / `on-inverse` | `#181713` / `#FAF9F6` | `#F3F0E9` / `#151513` | the opposite of the page |
| `color.inverse-accent` | `#7EB8F3` | `#2160AB` | the other mode's accent |
| `color.ok` / `ok-bg` | `#1F6B33` / `#1F6B3314` | `#7FCB8E` / `#7FCB8E1F` | the desktop has no green; value kept |
| `color.work-bg` · `wait-bg` | 8 % of the status colour | 12 % | one tint rule, as `accent_bg` and `danger_bg` |
| `color.err-fill*` | `#EEE0DE` `#EBD9D7` `#E9D2CF` | `#352728` `#3C2A2B` `#442E2E` | kept (T-492, iOS Deny) |
| `color.on-err` | `#FFFFFF` | `#151513` | dark: the page colour |

## T-160 notes 1–5

1. **Light `err-bg`** — now 8 % (`#AA2A2714`), as `danger_bg()`; the badge/banner pairs pass (`docs/contrast.md`).
2. **Switch** — `switch.track-on/track-off/knob` (+ `color.switch-*`) added.
3. **Sidebar · popover · tooltip · wash** — `color.side*`, `color.popover*`, `color.tooltip*` and components `sidebar.*`, `popover.*`, `tooltip.*` added; `color.hover`/`pressed` now carry the desktop's `wash` and `press()` values.
4. **`prose` · `text2`** — two steps, two tokens: `color.prose` (reading text) and `color.text-soft` (`text2`).
5. **Member colours** — `color.member-0…7` (the index `theme::look()` gives each role). Primitives `color.raw.member.N.300/700`.

## Open: the switch's off track

`switch-off` is 1.69:1 on a white card (light) and 1.63:1 on `raised` (dark), below WCAG 1.4.11's 3:1 for a
control's boundary. It is the shipped desktop value, so the tokens carry it and the contrast check leaves the pair
out; a fix is proposed as T-720.

## Noted by the critic, left as shipped

- The dark tooltip (`#2F2E2C` on `#151513`) has a soft edge and no border; its text reads at well over 4.5:1.
- `badge.neutral-bg` is `color.sunken`; in dark that is `#1B1A19` on a `#151513` page, so a neutral badge on the page barely shows a fill. Platforms adopting `badge.*` should check it on their screens (T-717–T-719).
- `color.dot` (`#3F6FF5`) is the logo's dot, not a UI colour, so it doesn't show on product screens; `inverse-accent` is the other mode's accent on purpose.

## Other platforms — what changes when each adopts the tokens

None of them uses the generated files yet: each hard-codes values that match tokens 1.1.0
(site `src/styles/global.css`·`app.css`, iOS `CompactionViews.swift` `StatusColor`, Android `Theme.kt` + `CompactionViews.kt` `V1`).
Read 2026-11-10 from `main` of `equipe-site` and `equipe-app`.

| Role | Site now (L / D) | iOS now (L / D) | Android now (L / D) | Tokens 2.0.0 (L / D) |
|---|---|---|---|---|
| page | `#F6F5F1` / `#0A0A0C` | system background | `#F6F5F1` / `#0A0A0C` | `#FAF9F6` / `#151513` |
| card · surface | `#FFFFFF` / `#16171A` | `#FFFFFF` / `#16171A` | `#FFFFFF` / `#16171A` | `#FFFFFF` / `#1F1E1C` |
| raised | — | `#FFFFFF` / `#1B1C20` | `#EFEDE7` / `#1B1C20` | `#FFFFFF` / `#1F1E1C` |
| sunken · code | `#EAE7E0` / `#17171C` | `#EAE7E0` / `#111114` | `#EAE7E0` / `#111114` | `#F2F1ED` / `#1B1A19` |
| text | `#15161A` / `#EFEDE7` | system `.primary` | `#0A0A0C` / `#EFEDE7` | `#181713` / `#F3F0E9` |
| muted | `#4F5159` / `#A3A5AB` | `#4F5159` / `#A3A5AB` | `#3F4148` / `#A3A5AB` | `#62605A` / `#A2A09A` |
| accent | `#1D4ED8` / `#8FB0FF` | `#1D4ED8` / `#8FB0FF` | none | `#2160AB` / `#7EB8F3` |
| on-accent | `#FFFFFF` / `#0B1226` | `#FFFFFF` / `#0B1226` | none | `#FAF9F6` / `#151513` |
| primary button | `#15161A` / `#EFEDE7` | accent fill | `#0A0A0C` / `#EFEDE7` | `#181713` / `#ECEAE4` |
| hairline | `#0A0A0C24` / `#E9E6DF24` | `#0A0A0C24` / `#E9E6DF24` (diagrams), system separator | `#0A0A0C24` / `#EFEDE724` | `#0000001A` / `#FFFFFF1A` |
| strong border | `#74767D` / `#7C7E85` | — | — | `#86847E` / `#77756F` |
| ok | `#1F6B33` / `#7FCB8E` | same | same | unchanged |
| work | `#825600` / `#E2B865` | same | same | `#865901` / `#EABA67` |
| wait | `#A04610` / `#EC9A62` | same | same | `#B83A0B` / `#FF7A45` |
| err | — | `#AD2E27` / `#EE7B74` | `#AD2E27` / `#EE7B74`, theme `#A32A24` / `#E8837D` | `#AA2A27` / `#F2857D` |

What a person will notice: the dark page lifts from near-black `#0A0A0C` to `#151513` and every grey turns warm;
the link blue moves from a violet-leaning `#1D4ED8`/`#8FB0FF` to the desktop's `#2160AB`/`#7EB8F3`; "your turn"
orange gets louder in dark (`#FF7A45`). iOS fills buttons with the accent today, not the `action` colour — adopting
`button.primary-*` changes that too. Each platform is its own code ticket — site T-717, iOS T-718, Android T-719 — with designer QA; the desktop wiring is T-704.
