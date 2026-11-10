# Component rules v1

Nine: button · input · card · chip · badge · list row · sheet/modal · decision card · toast (chip and badge split from the eight requested). Each component goes **use → variants → states → size → tokens → platform differences → don'ts**. Renders: `components/<name>-light.png` · `-dark.png` (HTML source in `build/sheets/`, made by `npm run sheets`; the web reference implementation is `reference/components.css`). Every value comes from `tokens/component.json` (→ semantic tokens → primitives).

## Shared rules

**States** — everything pressable uses the same grammar.
| State | Expression | Tokens |
|---|---|---|
| Default | each variant's default fill·text | component tokens |
| Hover (only where there's a pointer: web·desktop·iPad pointer) | one layer of neutral overlay. Only the primary button has its own colour | `color.hover`, `button.primary-bg-hover` |
| Pressed | an overlay one step darker than hover. iOS·Android use this instead of the system press effect | `color.pressed`, `button.primary-bg-pressed` |
| Focus | ring 2px, offset 3px (desktop 2px). Shows **only for keyboard·assistive-technology focus** (`:focus-visible`) | `color.focus`, `focus.ring-*` |
| Disabled | opacity 0.4 + no action + `aria-disabled`/`disabled`. If a reason is needed, in words beside it | `opacity.disabled` |
| Loading | the label changes to an "…ing" wording and it's blocked like disabled (never a spinner alone). The width stays | — |
| Error | colour + icon/symbol + words, all three | `color.err` |

**Accessibility** — text 4.5:1, graphics 3:1 (all 116 pairs in `contrast.md` pass). Meaning is never carried by colour alone. Press areas at least web·iOS 44 / Android 48 / desktop 28 (pointer) — if the visible size is smaller, a transparent area makes up the rest.
**Motion** — three durations (`fast` 120 · `base` 160 · `slow` 200ms), one curve (`out-cubic`). Travel: popover 4 · card 6 · dialog 8. **Reduce motion** (web `prefers-reduced-motion`, iOS `accessibilityReduceMotion`, Android system animation scale 0, desktop macOS Accessibility): no travel; all that remains is an opacity fade of 100ms or less.
**Type** — the role names in this table (`label`, `ui`…) are `type.*` in `tokens/semantic.json`. The apps use the platform font and follow Dynamic Type on iOS and sp scaling on Android — never cut text with a fixed height (minimum heights only).
**Language** — copy is short, in each language's chosen register (Korean: the polite -yo register, haeyo-che), the name is `equipe` (brand.md). Button labels are verbs: "Allow", "Decline" (never yes/no).

## 1. Button — `components/button-*.png`
- **Use**: starts one action. An area has *one primary button*.
- **Variants**: `primary` (`color.action` fill — a text-coloured fill, the product UI's primary action) · `secondary` (border `border-strong`) · `ghost` (link-coloured text, no fill — third-rank actions) · `danger` (`err` fill — actions that can't be undone) · `danger-soft` (opaque `err-fill` + `err` text — a refusal next to a primary, like Deny beside Allow; hover·pressed use `err-fill-hover`·`err-fill-pressed`, not the overlays; safe on translucent bars) · `brand` (`accent` blue fill — **web marketing only**, never in product UI. Why: so one action like approving isn't lost, T-144 guide §2.2).
- **Size**: md = `size.button`, sm = `button.height-sm`. Horizontal padding `button.padding-x`.
- **Tokens**: `button.*` (`tokens/component.json`).

| | Web | Desktop | iOS | Android |
|---|---|---|---|---|
| Height md / sm | 44 / 36 | 28 / 24 | 44 / 36 | 48 / 36 (touch area 48) |
| Horizontal padding | 20 | 12 | 16 | 24 |
| Corners | 8 | 8 | 12 (continuous curve `.continuous`) | 12 |
| Pressed | overlay | overlay | overlay (instead of the system highlight) | overlay (instead of the ripple — ripple colour set to `pressed`) |
| Text | `label` 16/600 | 14/600 | 15/600 (subheadline) | 14/600 (labelLarge) |

- **Don't**: two primary buttons in an area · when two are stacked on mobile, put the risky one *on top* (primary on top, cancel below), and for a risky action the danger button takes the primary place · labels of two or more lines · leave out the label (accessible name) on an icon-only button.

## 2. Input — `components/input-*.png`
- **Use**: one line of text input (names·codes). Multi-line uses the same tokens, only the height is `auto`.
- **Construction**: label (always visible, `label` type) → field → help text (`small`). A placeholder is an example, not a label.
- **States**: default · hover · focus (border 2px `focus` — the border thickens rather than a ring) · error (border `err` + `✕` + words) · disabled.
- **Size**: `size.field` — web 44 · desktop 36 · iOS 44 · Android 48. Type `ui`; **on web·iOS, under 16px the screen zooms on focus**, so web inputs are 16px (`ui`'s web value 16 ✓).
- **Platforms**: iOS draws the border itself on a `TextField` + `.textFieldStyle(.plain)`; Android takes only the colours of `OutlinedTextField` from tokens (no Material label animation; the label stays fixed on top — the same behaviour as the web).
- **Don't**: use the hairline (`line`) as a field border (below 3:1) · let the placeholder stand in for a format description · show an error by colour alone.

## 3. Card — `components/card-*.png`
- **Use**: one bundle of information. Variants: default (`surface` + hairline) · floating (`raised` + `elevation.float` — only for what floats above, like popovers·menus·toasts) · pressable card (default + overlay + focus).
- **Padding**: `card.padding` web·desktop 24 · mobile 16. Corners `radius.card` web·desktop 12 · mobile 16.
- **States**: only cards that open on a press have hover·pressed·focus. Information cards have no states.
- **Platforms**: an iOS card is `.background(colors.surface, in: RoundedRectangle(cornerRadius:, style: .continuous))`, with a hairline instead of a shadow (shadows don't show in dark).
- **Don't**: cards inside cards · different corners·shadows per card · build hierarchy with shadows (use hairlines and fill colour).

## 4. Chip — `components/chip-*.png`
- **Use**: filtering·choosing (picking among several). A pill (`radius.chip`). Variants: default · selected (`accent-bg` + `accent` text·border + a leading `✓`).
- **Size**: height 32 (desktop 24), horizontal 12, type `small`. On mobile the press area reaches 44/48 with a transparent area.
- **Don't**: use one to show a status (→ badge) · show selection by colour alone · put a long sentence in a chip.

## 5. Badge — `components/badge-*.png`
- **Use**: a status read at a glance. Five kinds: `ok` done · `work` working · `wait` your turn · `err` error · `neutral`.
- **Construction**: **a symbol + words, required** (✓ Done, ⋯ Working, ! Your turn, ✕ Error). The symbols come from the platform icon set (Lucide/SF Symbols/Material Symbols) — the sheet's Unicode glyphs are placeholders. The fill is the semantic colour at 12% + text in the semantic colour — 4.5:1 on any background (`contrast.md`'s badge rows).
- **Size**: height 22 (desktop 20), type `meta` 600, horizontal 8.
- **Fixed meanings**: amber (work) is "an agent is working", orange (wait) is "waiting on the person". **Amber/orange are never project·member colours** (exactly the desktop's `PROJECT_HUES` rule).
- **States**: no badge acts on a press (if it must be pressed, it's a chip or a button).
- **Counts (desktop sidebar·end of list rows)**: one count pill *filled* with `wait` per screen — only the total of what waits on the person (needs checking). Its breakdown (counts per project, progress, "your turn N") is `muted` numbers, and what they count is said by the **dot** in front (6pt, status colour, graphics 3:1). When one item waits (a conversation·a member's face·a project) it's the same orange dot, with the text beside it in ink (`text2`, medium). The count sits centred in a 22pt slot at the row's end, the dot in one line before it — rows line up whether it's 1 or 10. Tooltips·accessible names say in words what the number is. Why: when nearly every project waits, an orange number repeated on every row loses the signal (principles 1·2, brand §2.1·§4). More in `docs/DESIGN-SYSTEM.md` "Counts and what waits".

## 6. List row — `components/row-*.png`
- **Use**: a list of items of one kind (members, tickets, devices). Leading (avatar/dot) · a two-line body (title `ui`, secondary `small`) · trailing (badge/value/›).
- **Height** `size.row`: web 48 · desktop 32 · iOS 44 · Android 56. Horizontal padding 16 (desktop 12). Divider `line` 1px (none after the last row). Desktop rows have corners of 8 (the hover fill is rounded), the rest 0.
- **States**: hover·pressed (overlay) · selected (`accent-bg`, **text colour unchanged** — selection is never told by a fill alone but with a check·weight·`aria-selected`) · focus · disabled.
- **Platforms**: iOS draws its own dividers with `LazyVStack` instead of `List` (the system's inset dividers disagree with the tokens), and keeps the system's swipe actions. Android uses `Row` + `Modifier.clickable` (ripple colour `pressed`) instead of `ListItem`.
- **Don't**: two or more buttons in a row (press areas collide) · long text in the trailing slot · dividers and card borders together.

## 7. Sheet / Modal — `components/sheet-*.png`
- **Use**: stops what was under way to take one decision·input. Web·desktop: a centred dialog (max `sheet.max-width` 480). Mobile: a bottom sheet (grabber 36×4, top corners 24; use the system sheet `presentationDetents`/`ModalBottomSheet`, with colours·corners from tokens).
- **Construction**: title (`title`) → description (`body`) → actions (right-aligned; on mobile stacked, full width). At most two actions + close.
- **States**: open (dialog `slow` 200ms + 8 travel, the sheet on the system curve) · always a way to close (cancel button · tap outside · Esc/back) · processing (actions disabled + words).
- **Behind**: `color.scrim`. Focus is kept inside the sheet and goes back to where it was opened when it closes.
- **Don't**: sheets on sheets · a modal with no way to close · a modal for a simple notice (→ toast/banner) · default focus on the risky action.

## 8. Decision card — `components/decision-*.png` (equipe's own)
- **Use**: the moment the person's approval is needed (permission requests·your turn). **The component that must show most in the product** (playbook A2 principle: approval shows the most).
- **Construction** (top→bottom): badge `! Your turn` + time → who wants what (`title`) → where (`small`, muted) → the actual content (code block `sunken`) → a one-line note ("Allowed this once…") → two actions (same width, **decline = secondary · allow = primary**, decline on the left).
- **Surface**: border 2px `wait` (the your-turn colour). Once resolved the border goes back to `line` and the badge changes to `✓ Allowed` — it doesn't vanish but stays as a record.
- **States**: default · focus (the allow button) · processing (both buttons disabled, the allow label "Sending…") · done · error (a banner inside the card, buttons enabled again).
- **Platforms**: on iOS it rises as a sheet (`ApprovalSheetView`) with the same construction inside, one haptic on done. Android: a bottom sheet. Desktop: inline in the conversation + a notification (the notification never carries the content — only who needs what, the `desktop-cmdk-palette` decision).
- **Don't**: move default focus to allow *automatically* · make both buttons the same size·colour · hide the code content (never behind "details" — the grounds for judging come first) · the same approval as a toast.

## 9. Toast — `components/toast-*.png`
- **Use**: a light confirmation of what was just done ("Copied"). Only information that's no loss if it goes.
- **Surface**: the page's opposite colour (`inverse` / `on-inverse`, action `inverse-accent`) — reads over any background. Corners `radius.control`, shadow `elevation.float`, max width 520.
- **Time**: `motion.duration.toast` 5 seconds, held while hovered·focused·being read by a screen reader. With an action, keep it until closed or leave another way (history·settings). `role="status"` (iOS `UIAccessibility.post(.announcement)`, Android `LiveRegion.Polite`).
- **Position**: web·desktop bottom centre (`space.4` from the screen's edge), iOS·Android above the bottom tab bar/composer.
- **Don't**: errors as toasts (→ banner·field error, since they mustn't be missed) · two or more actions in a toast · stacking (at most one; a new one replaces the last).

## Appendix — names across platforms (token → code)

| Token | CSS | Rust (`tokens.rs`) | Swift | Kotlin |
|---|---|---|---|---|
| `color.surface` | `var(--eq-color-surface)` | `palette(dark).color_surface` | `colors.colorSurface` | `colors.colorSurface` |
| `space.4` | `var(--eq-space-4)` | `SPACE_4` | `EquipeSpace.s4` | `EquipeSpace.s4` |
| `radius.card` | `var(--eq-radius-card)` | `RADIUS_CARD` | `EquipeRadius.card` | `EquipeRadius.card` |
| `type.body` | `font: var(--eq-type-body)` | `TYPE_BODY` | `EquipeType.body.font` | `EquipeType.body.toTextStyle()` |
| `motion.duration.fast` | `var(--eq-motion-duration-fast)` | `MOTION_DURATION_FAST_MS` | `EquipeMotion.durationFast` (seconds) | `EquipeMotion.durationFast` (ms) |
| `button.primary-bg` | `var(--eq-button-primary-bg)` | `palette(dark).button_primary_bg` | `colors.buttonPrimaryBg` | `colors.buttonPrimaryBg` |

Rule: names are the token path as is (dots → CSS `-`, Rust `_`, Swift·Kotlin camelCase). Steps starting with a digit get letters: `space` → `s4`, `border` → `w1`. Units: CSS px · Rust logical px (f32)·time ms · Swift pt (CGFloat)·time seconds · Kotlin dp/sp·time ms. Colours: CSS `#RRGGBB(AA)` · Rust `0xRRGGBBAA` · Swift `Color(0x…)` · Kotlin `Color(0xAARRGGBB)`.
