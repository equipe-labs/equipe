---
name: ui-accessibility
description: Use on any screen you design, build or check — WCAG 2.2 AA as concrete rules, and how to check it.
---

# Accessibility

WCAG 2.2 level AA is the floor, not a bonus. Most of it is decided in design — contrast, targets, labels, order,
states — and the rest in code. Check it in the browser's accessibility tree, with axe, or in a simulator; never by
switching on VoiceOver, Zoom or other accessibility settings of the host Mac.

## Perceive

- **Contrast** (1.4.3, 1.4.11): text 4.5:1, large text 3:1; icons, input borders, focus indicators and chart marks
  that carry meaning 3:1 against their neighbours. Computed, in light and dark.
- **Not colour alone** (1.4.1): status, errors, links in text, chart series and required fields are also told by
  text, an icon, an underline or a shape.
- **Text alternatives** (1.1.1): every meaningful image has alt text saying what it shows or does; decorative images
  are hidden (`alt=""`, `aria-hidden="true"`); icon-only buttons have an accessible name.
- **Resize and reflow** (1.4.4, 1.4.10): at 200% text size and at 320 CSS px wide, no content or control is lost or
  clipped and nothing needs scrolling in two directions (tables and maps excepted).
- **Text spacing** (1.4.12): the layout survives increased line, paragraph, letter and word spacing — no fixed-height
  boxes around text.
- **Media**: captions for speech, transcripts for audio, a still for video loops; nothing plays sound on its own.
- **Language** (3.1.1): the page declares its language (`lang`); passages in another language declare theirs.

## Operate

- **Keyboard** (2.1.1, 2.1.2): everything works with the keyboard, in an order that follows the layout, with no
  traps; custom widgets follow the WAI-ARIA Authoring Practices (arrow keys in menus, tabs, listboxes and grids; Esc
  closes; Enter and Space activate).
- **Focus visible** (2.4.7) and **not obscured** (2.4.11): a clear focus ring on every focusable element
  (`:focus-visible`, about 2px, offset, 3:1); sticky headers, banners and cookie bars never cover it.
- **Focus management**: a dialog moves focus in, traps it, and returns it to the trigger on close; after deleting an
  item focus goes to a sensible neighbour; route changes move focus to the new heading.
- **Skip link and headings** (2.4.1, 2.4.6): a "Skip to content" link; one `h1` per page and a heading order without
  gaps; landmarks (`header`, `nav`, `main`, `footer`).
- **Target size** (2.5.8): at least 24×24 CSS px, or spaced so a 24px circle around each doesn't touch another; on
  touch platforms 44pt / 48dp.
- **Dragging** (2.5.7): anything done by dragging can also be done with single clicks or taps.
- **Timing** (2.2.1, 2.2.2): time limits can be extended; moving content over five seconds can be paused.
- **Page titles** (2.4.2): each view has a title saying where you are.

## Understand

- **Labels and instructions** (3.3.2): visible labels tied to their controls; format hints before the field.
- **Errors** (3.3.1, 3.3.3): named in text next to the field, with how to fix it; announced to screen readers.
- **Redundant entry** (3.3.7): information already given in the flow is filled in or offered, not asked again.
- **Accessible authentication** (3.3.8): sign-in needs no memory test or puzzle; paste and password managers work.
- **Consistent help** (3.2.6) and **navigation** (3.2.3): help and navigation sit in the same place across pages.
- **No surprise context changes** (3.2.2): changing a field doesn't submit the form or jump to another page.

## Robust

- **Semantics before ARIA**: `button`, `a`, `label`, `input`, `select`, `table`, `dialog`, lists and headings first;
  ARIA only where no element exists, and then the full pattern (role, state, keyboard).
- **Name, role, value** (4.1.2): every control exposes what it is, what it's called and its state (`aria-expanded`,
  `aria-pressed`, `aria-checked`, `aria-current="page"`).
- **Status messages** (4.1.3): toasts, inline validation and result counts announced politely (`aria-live="polite"`
  or `role="status"`); `assertive` only for urgent errors.
- **Tables**: header cells (`th` with `scope`), a caption or accessible name; not used for layout.
- **Forms**: errors linked with `aria-describedby` and `aria-invalid`; groups of radios in a `fieldset` with a
  `legend`; `autocomplete` on personal data fields (1.3.5).

## Mobile apps

- Every control has an accessibility label and the right trait or role; images are labelled or hidden.
- The screen works at the largest Dynamic Type / font-scale setting; nothing is fixed-height around text.
- Reading order follows the layout; custom gestures have an accessible action.
- Check with the simulator's Accessibility Inspector, or VoiceOver and TalkBack inside the simulator or emulator.

## How to check

1. Run axe (the precheck and `equipe design lint --render` do) and fix what it finds.
2. Walk the screen with the keyboard only: every control reached, in order, focus always visible, Esc closes.
3. Read the accessibility tree: every control named, roles right, nothing decorative exposed.
4. Render at 200% text and at 320px wide; look for clipping and overlap.
5. Compute contrast for every text and UI pair you introduced, in both themes.
6. Mark what you couldn't check as unconfirmed, with how to confirm it.

## Review checklist

- [ ] Contrast computed: text 4.5:1 (large 3:1), meaningful UI parts 3:1, both themes.
- [ ] Nothing told by colour alone.
- [ ] Meaningful images and icon-only controls named; decoration hidden.
- [ ] Nothing lost at 200% text or 320px wide; text spacing doesn't break boxes.
- [ ] Keyboard reaches everything in order; focus visible, never covered; dialogs trap and return focus.
- [ ] Skip link, one `h1`, ordered headings, landmarks, a title per view, `lang` set.
- [ ] Targets at least 24px (44pt / 48dp on touch); dragging has an alternative.
- [ ] Labels tied to controls; errors in text with a fix; no repeated entry; sign-in without memory tests.
- [ ] Native elements first; ARIA patterns complete; live regions for status messages.

Sources: WCAG 2.2 and its Understanding documents (W3C); WAI-ARIA Authoring Practices; Apple Human Interface
Guidelines; Material Design 3; Vercel's Web Interface Guidelines (MIT) — in our own words.
