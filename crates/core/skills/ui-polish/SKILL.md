---
name: ui-polish
description: Use before you hand in UI — focus, keyboard, loading, layout stability, edge-case content and details.
---

# Production polish

The difference between a screen that demos well and one that ships well is in the cases nobody draws: the slow
network, the 60-character name, the second click, the keyboard user, the dark theme. Go through them before hand-in.

## Focus and keyboard

- A visible focus ring on everything focusable: `:focus-visible`, about 2px, offset 2px, 3:1 against its
  surroundings. Never `outline: none` without a replacement.
- Sticky headers and footers never cover the focused element or an anchored heading (`scroll-margin-top`).
- Tab order follows the visual order; Esc closes the topmost overlay; Enter submits; arrow keys move within menus,
  tabs, lists and grids.
- Shortcuts appear in menus and tooltips with the platform's symbols (`⌘` on macOS, `Ctrl` elsewhere) and work on
  non-QWERTY layouts.
- Links navigate (`<a>`, so Cmd/Ctrl-click opens a tab); buttons act. Never a clickable `div`.

## Targets

- The hit area matches what looks clickable, and is at least 24px (44px on touch) even when the icon is 16px.
- No dead zones: the whole row, the label and the checkbox share one target; the gap between a label and its control
  is clickable.

## Loading and perceived speed

- **Response limits**: under about 100ms feels instant; up to about 1s keeps the flow but wants a sign of work beyond
  a few hundred ms; past about 10s people need progress with a percentage or steps, and a way to cancel.
- **Don't flash**: wait about 150–300ms before showing a spinner or skeleton, and once shown keep it at least about
  300–500ms, so fast responses don't flicker.
- **Skeletons** for page areas whose shape is known, matching the final layout exactly; a **spinner inside the
  control** for a button's action, keeping its label ("Saving…"); a **progress bar** for long jobs.
- **Optimistic updates** when success is likely and the action is reversible (a toggle, a rename, a reorder): show
  the result at once, reconcile with the server, and on failure roll back with a clear message or offer Undo.
- **Keep what's there**: refresh in the background without blanking the page; keep scroll position; restore it on
  Back.
- **Writes** (save, delete, update) aim to finish in under 500ms; prefetch the likely next view.

## Layout stability

- Reserve space for images, embeds and ads (`width`/`height` or `aspect-ratio`); fonts with metric-matched fallbacks
  so text doesn't jump on load.
- Hover, selection and loading never change an element's size: swap colour or shadow, keep a transparent border of the
  same width, keep the label in a loading button.
- `scrollbar-gutter: stable` where a modal or late content would toggle a scrollbar; only useful scrollbars show.
- Core Web Vitals at the 75th percentile: LCP ≤ 2.5s, INP ≤ 200ms, CLS ≤ 0.1.

## Edge-case content

Test each list, card and table with:

- a very long name or title (truncate with `…` and show the full value, or wrap — decided, not accidental), a very
  short one, an empty one;
- zero, one, many and a huge number of items (pagination or virtualisation past a few hundred rows);
- big, negative and zero numbers (`1,284,503`, `−12.5%`, `0`), and currencies in the locale's format;
- a missing avatar (initials on a colour derived from the name), a missing image (a fixed-ratio placeholder), a broken
  link;
- translation 40% longer, right-to-left text, emoji and accented characters in names;
- slow network, offline, a timeout, an expired session, no permission.

## Forms and actions under real use

- Double clicks don't double submit; requests carry an idempotency key.
- Unsaved changes warn before leaving; drafts survive a reload where losing them hurts.
- Paste works everywhere; autofill styling matches the design; one-time codes paste and autofill.
- Destructive actions confirm or offer Undo.
- State that people would share or come back to lives in the URL.

## Details that show

- `<title>` per view; favicon legible on light and dark tabs; `theme-color` and `color-scheme` set.
- Native `select` and scrollbars legible in dark mode; selection colour and placeholder colour set.
- Icons from one set, one stroke width, aligned optically with the text beside them; images sharp at 2× (`srcset`).
- Nested radii parallel, shadows layered and soft, borders semi-transparent on tinted surfaces.
- Modals and drawers contain their scroll (`overscroll-behavior: contain`); the page behind doesn't scroll.
- Tooltips: a delay for the first, none for the next; never essential.
- Dark mode: no white images or screenshots glaring on dark surfaces; check every screen in both themes.

## Review checklist

- [ ] Focus visible and never covered; keyboard order, Esc, Enter and arrows work; links are links.
- [ ] Hit areas at least 24px (44px touch); no dead zones.
- [ ] Loading: delayed and minimum-duration indicators; skeletons match the layout; buttons keep their label.
- [ ] Optimistic updates roll back on failure; background refresh keeps the page and scroll.
- [ ] No layout shift from images, fonts, hover or loading; CLS ≤ 0.1, LCP ≤ 2.5s, INP ≤ 200ms.
- [ ] Long, short, empty, zero, many, huge, translated and right-to-left content all hold.
- [ ] Offline, timeout, expired session and no-permission states handled.
- [ ] No double submits; unsaved changes protected; paste and autofill work.
- [ ] Titles, favicon, theme colour, dark-mode natives, icon consistency and image sharpness checked.

Sources: Vercel's Web Interface Guidelines (MIT); web.dev on Core Web Vitals and layout shift; Nielsen Norman Group on
response-time limits and skeleton screens; WCAG 2.2 — in our own words.
