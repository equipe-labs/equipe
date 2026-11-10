# Token proposals surfaced by applying T-160 to the desktop (to fix at the source, `tokens/`)

The generated files weren't edited; what follows was left in the desktop as **local adjustments**.

1. **`color.err-bg` (light, 12% alpha)** — hovering a danger (ghost) button over a card or code background puts the `err` text at 4.4:1, short of AA (4.5), because sunken darkened from `#ECEBE6` to `#EAE7E0`. The desktop lowers the alpha ×0.65 on hover only (`ui.rs` `Kind::Danger`). Proposal: light `err-bg` alpha at ~8%, or a separate `button.danger.bg-hover` component token.
2. **Dark switch "on" colour (`switch_on` `#4A76A0`)** — the tokens have no switch track, so it stays local. Proposal: `switch.on/off/knob` component tokens.
3. **Sidebar, tooltip and popover surfaces** (`side`, `side_hover`, `side_sel`, `popover`, `tip_*`) and `wash` (hover surface) — no token matches, so they stay local. `color.hover/pressed` exist but with different values (dark `0xEFEDE712` etc.). Proposal: add `color.side*`, `color.popover`, `tooltip.*`, or document that they're local on purpose.
4. **`prose`, `text2`** (reading and secondary text steps) — the tokens have only `text`/`muted`. Proposal: `color.text-soft`.
5. **The eight member/project colours** — no tokens (left alone, per adoption.md §2 risk ⑤).
6. **`ICON_SM` 14→16** (`SIZE_ICON_SM`) is applied, and screenshots showed no sidebar spacing problem. Most desktop icons are 16 — with the token named `icon-sm` it takes the same value as `ICON`, so the two overlap. Proposal: `size.icon-xs` if the desktop needs 14px.
7. Reduce Motion (`MOTION_DURATION_REDUCED_MS`, a 100ms fade) isn't applied yet (the end state shows at once) — optional, left for later.

## 2026-11-10 — merging with main (T-160 rework)
- Main's desktop palette was reworked on its own after T-160 (a single warm neutral; dark bg `#151513`, accent `#7EB8F3`, orange `#FF7A45`; light bg `#FAF9F6`, accent `#2160AB`, etc.). The `design/` tokens (v1.1.0) didn't follow (dark bg `#0A0A0C`, accent `#8FB0FF`). Filling the palette from the tokens would undo main's rework, so **the palette keeps main's values**, and only spacing, radius, size, type size, line height, motion and shadow come from the tokens.
- The person's pick of "dark primary button = bone" and the v1 blue is met: main's new palette already uses a bone primary button (`#ECEAE4`) and a blue accent, though not the v1 tokens' exact values.
- **Token-side proposal (first):** update `design/tokens/semantic.color.*` to main's desktop palette (or the person decides to regenerate the palette from the tokens). Then `theme.rs`'s `Palette` can be filled from `tokens::palette()`.
- `ICON_SM` stays a local 14, as main's screens use 14 (the token `SIZE_ICON_SM` is 16).
- The local hover fix for danger buttons is no longer needed: main's `danger_bg` (8%) already solves it.
