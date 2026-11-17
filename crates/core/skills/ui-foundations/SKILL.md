---
name: ui-foundations
description: Use when you set or judge space, grid, density, type or colour.
---

# Foundations: space, grid, density, type, colour

These are the numbers a screen stands on. Where the project's design system already sets them, its tokens win and
these rules say how to use them; where it has none, start from here and put the values in the tokens, never inline.

## Space

- **One scale, on a 4px grid**: 4, 8, 12, 16, 24, 32, 48, 64, 96, 128. Every gap, padding and margin is a step of it.
  A value off the scale (13px, 22px) is a finding unless it is optical correction of 1–2px.
- **Proximity carries grouping**: the gap inside a group is smaller than the gap between groups — label to its field
  4–8, field to field 16–24, group to group 32–48, section to section 48–96 (landing 96–160). If two gaps are
  equal, the eye can't tell what belongs together.
- **Start roomier, then tighten**: lay out with too much space and remove it until it feels right; cramped is harder
  to rescue than airy.
- **Separate with space before lines**: reach for a border or a background only when space and alignment can't do
  it. Two levels of borders or surfaces at most in any region.
- Do: `padding: var(--space-4) var(--space-6)` (16/24). Don't: `padding: 14px 22px`, or the same 16px between
  everything.

## Grid and width

- **Landing**: 12 columns, content at 1120–1280px max, gutters 24–32px; side margins 16–20px on phones, 24–32px on
  tablets, at least 32px on desktop.
- **Web product**: a fixed sidebar and fluid content; tables and boards use the full width; forms, settings and
  reading sit at 640–720px; a detail panel at 360–480px.
- **Measure**: body text runs 45–75 characters a line (aim for 60–70); in a wide window, cap the text column rather
  than letting a line cross the screen. Nothing thin should float lost in a 1920 window either — widen the layout into
  columns or centre a capped container.
- **Breakpoints follow the content**: break where the layout breaks, typically around 640, 768, 1024, 1280 and 1536.
  Check 320 (reflow), 360/390, 768, 1024, 1280, 1440, 1920.
- **Align to few lines**: every element lines up with something — a column edge, a baseline, another element's
  edge. A new element joins an existing line instead of starting its own.

## Density

| | Compact (dense tools) | Default | Comfortable / touch |
|---|---|---|---|
| Table row | 32px | 40px | 48px and up |
| Control height | 28–32px | 36px | 44pt / 48dp |
| Body text | 13px | 14px | 16–17px |
| Base gap | 8px | 12px | 16px |

Pick one density per product and keep it. A density toggle is for tables people live in, not a substitute for
choosing.

## Type

- **A scale with a ratio**: about 1.125–1.2 for products, 1.25–1.333 for landings. A workable set: 12, 13, 14, 16,
  18, 20, 24, 30, 36, 48, 60, 72. Two or three sizes per block; more than six on a screen is a finding.
- **Hierarchy through weight and colour before size**: primary text at full contrast, secondary one step lighter,
  tertiary for metadata; 400 for text, 500–600 for labels and headings. Three weights at most.
- **Line height**: 1.4–1.6 for body, 1.2–1.3 for headings, 1.0–1.15 for large display; tighter as size grows.
- **Tracking**: slightly negative on large display (about −0.01 to −0.03em), normal for body, slightly positive for
  small all-caps labels (+0.02 to +0.06em) — and use all caps sparingly.
- **Numbers**: tabular figures (`font-variant-numeric: tabular-nums`) wherever numbers line up — tables, KPI tiles,
  prices, timers; proportional figures in running text. One decimal precision per column.
- **Breaks**: balance headings (`text-wrap: balance`), avoid single-word last lines in paragraphs (`text-wrap:
  pretty`), keep units and names together with a non-breaking space (`10 MB`), use the real ellipsis `…` and curly
  quotes.
- **Scripts**: choose a family that covers the product's languages, with a fallback stack per script. Scripts set
  without spaces (Chinese, Japanese, Korean) want more line height (about 1.6–1.8), no negative tracking and their
  own line-breaking rules; right-to-left scripts mirror the layout.
- **Loading**: two families at most (one is often enough), subset to what the product uses, preloaded for the first
  screenful, with a metric-matched fallback so text doesn't jump.
- Do: a 14px table with 12px tertiary metadata and 600 headers. Don't: 11px grey text for anything a person needs to
  read, or five sizes inside one card.

## Colour

- **Roles, not hues**: background, surface, raised surface, border (subtle and strong), text (primary, secondary,
  tertiary, disabled, on-accent), accent (primary action, selection, focus ring), status (success, warning, danger,
  info — each with text, background and border), and a separate palette for charts. Screens use roles; roles map to a
  scale.
- **Neutrals carry the screen**: a 10–12 step neutral scale, faintly tinted toward the brand hue, does most of the
  work; the accent covers well under a tenth of a product screen.
- **Contrast (WCAG 2.2 AA)**: text 4.5:1; large text (24px, or about 18.7px bold, and up) 3:1; icons, input borders, focus
  rings and chart marks that carry meaning 3:1 against what is next to them. Compute it — never judge by eye. A
  disabled control is exempt, but it should still be legible.
- **State raises contrast**: hover, pressed and focus are stronger than rest, never fainter.
- **Status**: never colour alone — pair it with an icon, a label or a shape. Red means danger or error only; don't
  spend it on a brand flourish. Up isn't always good: colour a delta by whether it is good for the person, not by its
  sign.
- **Dark mode is its own palette, not an inversion**: a near-black base (about #0B0B0C–#121212, not pure black),
  surfaces get lighter as they rise, accents a little less saturated, shadows replaced by lighter surfaces and
  borders, every pair's contrast checked again. Set `color-scheme` and `theme-color` so the browser's own parts match.
- **Tint toward the hue**: on a coloured background, tint its borders, shadows and secondary text toward the same hue
  rather than grey.
- Do: `color: var(--text-secondary)` on `var(--surface)`, checked at 4.5:1 in both themes. Don't: `#999` on white
  (2.8:1), or the brand colour as the only sign of an error.

## Surfaces, radius and shadow

- **Radius scale**: e.g. 4, 6, 8, 12, 16 and full. Small controls take the small steps; cards and sheets the larger. A
  nested element's radius is the outer radius minus the padding between them, so the curves run parallel.
- **Shadows**: two layers (a tight one for contact, a soft one for ambient light), low opacity; a 1px semi-transparent
  border sharpens the edge. Elevation means something — a menu above a card above the page — not decoration.
- **One treatment per job**: the same kind of thing looks the same everywhere; a second card style needs a reason.

## Review checklist

- [ ] Every spacing value is on the scale; gaps inside groups are smaller than gaps between them.
- [ ] Everything aligns to a few shared lines; nothing floats off-grid.
- [ ] Text columns sit at 45–75 characters; the width is used sensibly at 1280–1920.
- [ ] One density for the product, inside the ranges above for its kind.
- [ ] Two or three type sizes per block, three weights at most; hierarchy reads at a squint.
- [ ] Tabular figures wherever numbers line up; units glued with non-breaking spaces.
- [ ] Colours come from roles in the tokens; the accent is rare; status never by colour alone.
- [ ] Contrast computed: text 4.5:1 (large 3:1), meaningful UI parts 3:1, in light and dark.
- [ ] At most two levels of borders or surfaces; nested radii run parallel.

Sources: Refactoring UI's widely summarised principles (space before borders, hierarchy through weight and colour,
start roomy); WCAG 2.2; Vercel's Web Interface Guidelines (MIT); IBM Carbon; Material Design 3 — in our own words.
