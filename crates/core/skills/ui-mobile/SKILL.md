---
name: ui-mobile
description: Use for phones — platform conventions, navigation, touch and thumb reach, safe areas, onboarding and settings.
---

# Mobile apps and narrow screens

The first part is for mobile apps (native or cross-platform). The last section is for web products and landings at
phone widths.

## Platform first

- Follow the platform the app runs on. iOS: Apple's Human Interface Guidelines; Android: Material 3. Where a
  cross-platform app shares one design, share the brand (colour, type, voice, illustration) and let navigation,
  controls, back behaviour and system sheets follow each platform.
- Use the platform's components before custom ones: navigation bars, tab bars, lists, switches, pickers, sheets,
  alerts, share sheets. A custom control has to beat the native one at something the person needs.
- Respect the system: Dynamic Type / font scale, dark mode, reduced motion, bold text, the system back gesture.

## Navigation

- **Top level**: a tab bar (iOS) or navigation bar (Android) of three to five destinations, labelled with an icon
  and a word; the current one marked by more than colour. Not a hamburger menu as the main navigation.
- **Depth**: push screens onto a stack; back is at the top leading corner (iOS) or the system back (Android), and an
  edge swipe goes back on iOS. Don't hijack either.
- **Focused tasks** open in a sheet or a full-screen modal with a clear "Cancel"/"Done" or close; a sheet can rest at
  medium or large height when the content underneath still matters.
- **Large titles** on top-level iOS screens collapse on scroll; detail screens use the standard title.
- The person always knows where they are and how to get back.

## Touch and reach

- **Targets**: at least 44×44pt on iOS and 48×48dp on Android, even when the visible icon is smaller; at least 8pt
  between neighbouring targets.
- **Thumb reach**: one-handed, the lower third of the screen is easy, the top corners hard. Put the primary action
  and frequent controls low (a bottom button, a tab bar, a sheet), keep rarely used and destructive actions out of
  the easy zone.
- **Gestures are shortcuts**: swipe actions on list rows, pull to refresh, long press — each also reachable by a
  visible control (WCAG 2.2, Dragging Movements).
- **Feedback within 100ms**: a pressed state on every tappable thing; a light haptic for confirmations and toggles,
  never for every tap.

## Layout and type

- One column; side margins 16–20pt; content in lists or grouped sections more than in cards.
- **Safe areas**: nothing interactive under the status bar, the notch or island, or the home indicator; full-bleed
  backgrounds may extend under them.
- **The keyboard**: the focused field and its submit action stay visible above it; the return key says what happens
  (Next, Search, Send, Done); the right keyboard for the field (email, number, phone).
- **Type**: the platform's styles — iOS body 17pt, footnote 13pt, smallest 11pt; Android body 14–16sp. Check the
  screen at the largest accessibility size: text wraps rather than truncates, and nothing overlaps.
- Images and media fill the width with a fixed aspect ratio so the layout doesn't jump as they load.

## Onboarding

- Let people reach the value fast: at most three short intro screens, skippable — or none, and teach in context.
- Delay sign-up until there is a reason (saving, syncing); offer the platform's sign-in (Sign in with Apple, Google)
  and passkeys where the product supports them.
- **Permissions in context**: ask for the camera when they tap "Scan", not at launch. If a short explanation helps,
  show it right before the system prompt, with a way to say "Not now". When denied, show what still works and how to
  turn it on in Settings.
- Progress is visible in multi-step setup ("2 of 3"); each step has one primary action at the bottom.
- Don't: a carousel of marketing claims, notifications asked for on the first screen, a forced account before anything.

## Settings

- A grouped list: sections with short headers, a footer under a section to explain an option, the most used first.
- Switches apply at once, with no Save button; choices with several options open a list with a checkmark on the
  current one.
- Account, sign out and delete account at the bottom, separated; deleting asks for confirmation and says what is lost.
- Large settings get search. Links to system settings (notifications, permissions) where the system owns the switch.
- Show the current value on the row ("Language · English") so people don't have to open it to know.

## Feedback and states

- Loading: skeletons for content, the platform's activity indicator for short waits, pull to refresh for lists that
  change.
- Offline: say so once (a banner), keep showing what's cached, queue what can wait.
- Errors: inline near the problem, with a retry; alerts only for what needs a decision now.
- Empty states: what goes here and the one action that fills it.

## Web at phone widths

- Design and check at 360 and 390 (plus 320 for reflow); nothing scrolls sideways except a table or carousel inside
  its own container.
- Touch targets at least 44px; inputs at 16px or more; `env(safe-area-inset-*)` respected for fixed bars.
- Navigation collapses to a menu or a bottom bar; the primary action stays in the first screenful.
- Hover-only affordances get a visible alternative; `:hover` styles only under `@media (hover: hover)`.

## Review checklist

- [ ] Platform navigation and components (tab bar, stack with back, sheets); no hamburger as the main navigation.
- [ ] Targets at least 44pt / 48dp, 8pt apart; pressed states on everything tappable.
- [ ] The primary action within thumb reach; destructive actions out of the easy zone.
- [ ] Safe areas respected; focused fields and their action above the keyboard; the right keyboard and return key.
- [ ] Platform text styles; the screen holds at the largest accessibility text size.
- [ ] Every gesture has a visible alternative.
- [ ] Onboarding short and skippable; permissions asked in context, with a path after a denial.
- [ ] Settings grouped, switches apply at once, current values shown, account actions at the bottom.
- [ ] Web at phone widths: no sideways scroll, 44px targets, 16px inputs, primary action in the first screenful.

Sources: Apple Human Interface Guidelines; Material Design 3; WCAG 2.2 (Target Size, Dragging Movements); Vercel's Web
Interface Guidelines (MIT); Mobbin's public skills (MIT) on reading the platform off the source — in our own words.
