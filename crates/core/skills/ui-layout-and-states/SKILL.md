---
name: ui-layout-and-states
description: Use when you lay out a screen — hierarchy, one primary action, disclosure, navigation, overlays and every state.
---

# Layout, hierarchy and states

## Hierarchy

- **Write the job first**: one line — who arrives, what they came to do, what done looks like ("a finance lead
  checking whether this month's spend is on budget; done when they know, or have opened the line that isn't"). The
  layout serves that line; whatever doesn't serve it goes lower or goes.
- **The eye lands on the first step of the job**: at a three-second glance and squinting, the most important thing
  is seen first, the second thing second. If everything is bold, nothing is.
- **One primary action per view**: one filled button. Other actions are secondary (outline or tonal) or tertiary
  (text or ghost). A second filled button in the same view is a finding.
- **Destructive is quiet until it is the action**: "Delete project" lives in a menu or a danger zone at the bottom;
  in its confirmation it becomes the primary, styled as destructive.
- **Rank with several tools at once**: position (top, left in left-to-right scripts), size, weight, colour and space
  around. De-emphasising the rest often works better than emphasising one thing more.
- **Things sit beside what they are about**: a label next to its value, an action next to the object it acts on,
  help next to the decision it informs — not a scroll or a tap away.

## When you add or change a part

- **Change the least that answers the problem**: rearrange before you restyle; parts the change doesn't need keep
  their place, size and look.
- **A new element copies its nearest sibling**: the same component, size, weight, colour role and spacing as the
  closest element doing the same kind of job. If a reader can tell which part is new by how it looks, it was built
  wrong.
- **It never outranks what it serves**: a new badge or hint sits below the thing it belongs to in weight.
- **Use the lines already there**: align to an existing edge, sit on an existing spacing step.
- **The same thing looks the same**: two elements doing one job get one treatment.
- **Don't add for the sake of adding**: no icon, badge, divider, shadow, panel, subtitle restating a heading or
  helper line under a self-explaining control, unless a named problem needs it.

## Progressive disclosure

- Show what most people need for the task; put the rest one step away — "More options", a disclosure, an advanced
  section, a detail panel.
- Never hide the primary action or information a decision depends on. Two levels of disclosure at most.
- Defaults carry the weight: a good default beats an exposed option.
- Don't: a settings page of forty equal toggles. Do: the five that matter, grouped, and "Advanced" below.

## Web navigation

- **Sidebar** for an app with more than about five sections or with depth; **top bar** for five or fewer; **tabs**
  for peer views of the same object (two to seven, never tabs inside tabs); **breadcrumbs** when the hierarchy is
  three levels or deeper; a **command palette** for a large app.
- The current place is always visible: the active item marked by more than colour (weight, an indicator bar).
- The URL holds the state: tabs, filters, sort, pagination and open panels survive refresh, share and Back.
- Navigation uses links (Cmd/Ctrl-click opens a new tab); actions use buttons.
- Mobile navigation is in `ui-mobile`.

## Overlays

| Need | Use | Notes |
|---|---|---|
| Short task that must be finished or cancelled | Modal dialog | Focus trapped and returned; Esc closes; never a modal on a modal |
| Details or edits while keeping the list in view | Side panel / drawer | 360–480px; the list stays usable |
| Small contextual choices | Popover / menu | Opens from its trigger; closes on outside click and Esc |
| Brief, non-critical confirmation | Toast | 4–6s, pauses on hover, offers Undo; never for an error needing action |
| Status of the whole page or account | Banner | Persistent until resolved or dismissed |
| A field's or block's own problem | Inline message | Beside what it is about |
| A label for an icon-only control | Tooltip | Not for essential information; first one after a short delay |

## States: design every one

A screen is not designed until each state it can be in is. For every view, list them and show each:

- **Empty, first use**: what this place is for, why it is empty, and the one action that fills it ("No invoices yet.
  Create your first invoice."). A sample or a short illustration only if it helps; never a dead end.
- **Empty, no results**: repeat what was searched or filtered, and offer to clear filters or broaden the search.
- **Empty, cleared**: the inbox-zero case — a short, calm confirmation.
- **Loading**: a skeleton shaped like the content for page areas, a spinner inside the control for an action; timing
  rules are in `ui-polish`.
- **Partial and sparse**: one row, one data point, half the fields filled — the layout still holds.
- **Dense and overflowing**: the 500-row table, the 80-character name, the 12-digit amount — truncation, wrapping,
  pagination or virtualisation decided.
- **Error**: what happened, in plain words, what to do next, and the person's input kept. Page-level failures get a
  page; a field's error stays at the field.
- **Disabled and read-only**: say why something can't be used, and how to get access, instead of a silent grey.
- **Offline, permission denied, expired session, rate-limited**: each has a message and a way forward.
- **Success**: confirm what changed, then get out of the way.

Every screen offers a next step or a way back — no dead ends.

## Review checklist

- [ ] The job is written in a line; the first thing the eye lands on is its first step.
- [ ] Exactly one primary action per view; destructive actions are quiet until confirmed.
- [ ] Information sits beside the decision it informs; nothing the job needs is missing.
- [ ] New or changed parts copy their nearest sibling and add no new alignment line.
- [ ] Advanced options are one step away, the primary never hidden; at most two levels of disclosure.
- [ ] The current place is visible; tabs, filters and panels live in the URL.
- [ ] The right overlay for each need; no modal on a modal; toasts never carry an error that needs action.
- [ ] Empty (first use, no results, cleared), loading, partial, overflowing, error, disabled and success states are
      all designed, with real copy and a next step.

Sources: Nielsen Norman Group on progressive disclosure, empty states and usability heuristics; Vercel's Web
Interface Guidelines (MIT); the job walk and "nearest sibling" rules of Mobbin's `redesign-screen` skill (MIT),
adapted; Atlassian and Polaris on overlays and messages — in our own words.
