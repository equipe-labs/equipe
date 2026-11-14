---
name: ui-motion
description: Use when anything moves or responds — durations, easing, hover and press, reduced motion.
---

# Motion and interaction

Motion earns its place by explaining something: that the press was felt, where a panel came from, what changed. Used
for its own sake it slows people down, and the more often they see it, the more it costs.

## When to animate, when not

- **Animate** state changes people need to notice (a row added, an item moved), the arrival and departure of overlays
  (so it is clear where they came from), direct manipulation (drag, swipe, pinch follow the finger at once), and the
  rare moment worth a little delight (a first success, a launch page).
- **Don't animate** what is triggered from the keyboard or done dozens of times a day (opening a command palette,
  switching tabs, opening a menu in a tool you live in), data refreshing in a dense tool, content that should simply
  be there on load, or anything that makes people wait to read.
- **Ask how often it will be seen**: once (onboarding, a landing reveal) can take its time; hourly must be near
  instant; constantly should not move at all.

## Durations

| What | Duration |
|---|---|
| Hover, press, toggle, focus | 100–150ms |
| Tooltip, dropdown, popover, small reveal | 150–200ms |
| Modal, drawer, sheet, panel | 200–300ms |
| Page or large-area transition | 250–400ms (mobile platforms around 300–350ms) |
| Landing reveal, once | 300–600ms |

Product UI stays under 300ms. Exits run a little faster than entrances (about 20–30% shorter): leaving shouldn't hold
anyone up.

## Easing

- **Entering and exiting**: ease-out — fast start, gentle stop — so the response is felt at once. Built-in `ease-out`
  is weak; a stronger custom curve such as `cubic-bezier(0.23, 1, 0.32, 1)` reads crisper.
- **Moving across the screen** (an element going from A to B while visible): ease-in-out.
- **Avoid ease-in for UI**: a slow start feels like lag.
- **Linear** only for progress bars, spinners and constant loops.
- **Springs** for gestures and anything that can be interrupted mid-flight; in productivity tools, damped so they
  barely overshoot.

## Concrete details

- **Press**: scale to about 0.97–0.98 on `:active` for buttons and cards that act; release snaps back.
- **Appear**: from opacity 0 and a scale of about 0.95 (or a few pixels of travel) — never from `scale(0)`.
- **Origin**: popovers, menus and tooltips grow from their trigger (`transform-origin` at the trigger's side), not from
  their centre.
- **Tooltips**: the first one in a group waits about 300–500ms; once one is open, neighbours open instantly with no
  animation.
- **Hover** effects only for pointers that hover (`@media (hover: hover) and (pointer: fine)`); touch gets the pressed
  state instead.
- **Hover never moves layout**: change colour, background, shadow or opacity — not weight, border width or size.
- **Stagger** lists by 20–50ms per item and cap the total around 300ms; never stagger rows in a working table.
- **Interruptible**: a toggle clicked twice reverses from where it is. CSS transitions do this; keyframe animations
  don't — use transitions for state.
- **Properties**: animate `transform` and `opacity` (and `filter` sparingly); never `width`, `height`, `top` or `left`
  for movement, and never `transition: all` — list the properties.
- **Skeleton shimmer**, when used, is slow (about 1.5–2s per sweep) and low-contrast.
- **Numbers** may count up on a landing page; in a product they just change, perhaps with a brief highlight.

## Reduced motion and safety

- Honour `prefers-reduced-motion: reduce`: replace movement with a short opacity fade (150ms or less) or nothing;
  turn off parallax, scroll-linked effects, autoplay and decorative loops. The floor checks there is a still version
  of whatever moves.
- Anything that moves, blinks or scrolls on its own for more than five seconds alongside other content can be paused,
  stopped or hidden (WCAG 2.2.2). Nothing flashes more than three times a second (WCAG 2.3.1).
- Video loops are muted, `playsinline`, pausable, with a still poster.

## Performance

- Aim for 60fps on a mid-range phone: transform and opacity run on the compositor; `will-change` only on what is
  about to move, removed after.
- Prefer CSS, then the Web Animations API, then a JavaScript library — and drive the main thread as little as possible.
- Test with CPU throttling and on a low-power device; long animation frames show up as jank.

## By product kind

- **Landing**: the motion budget is largest — one considered entrance per section, a product loop, scroll reveals
  that never hide content needed to understand the page.
- **Web product**: almost all motion is feedback and orientation, 100–200ms; nothing on load; nothing on keyboard
  actions.
- **Mobile app**: the platform's transitions and gestures, with direct manipulation that follows the finger; haptics
  sparingly.

## Review checklist

- [ ] Every animation explains something; nothing animates on keyboard actions or high-frequency actions.
- [ ] Durations inside the table; product UI under 300ms; exits faster than entrances.
- [ ] Ease-out for enter and exit, ease-in-out for on-screen movement, no ease-in, linear only for progress.
- [ ] Press feedback; appear from about 0.95, never 0; popovers grow from their trigger.
- [ ] Hover only on hover-capable pointers, and it never shifts layout.
- [ ] Only transform and opacity animate; no `transition: all`; animations interruptible.
- [ ] Reduced motion replaces movement with a fade or nothing; anything over five seconds can be paused; no flashing.

Sources: Emil Kowalski's articles on animation craft (emilkowal.ski); Rauno Freiberg's writing on interaction design
(rauno.me); Vercel's Web Interface Guidelines (MIT); Material Design 3 motion; WCAG 2.2 — in our own words.
