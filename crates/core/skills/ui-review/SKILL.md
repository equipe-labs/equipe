---
name: ui-review
description: Use when you judge a rendered screen — before the critic or second look, and on design QA.
---

# Reviewing a rendered screen

A review judges pictures, not intentions: the screen as drawn at each width, in each theme and state. Your own review
is a pre-check that catches what you can fix before fresh eyes see it — it never replaces the critic, the second
look or design QA, and it never grades your own work as done.

## What you need

- The pictures `equipe design lint --render` drew (360 to 1920), light and dark, and each state the screen has; on
  design QA, the precheck's pictures and the spec beside them.
- The references of the screen's patterns at the same width (`design-references`).
- The brief or ticket: what the screen is for and its acceptance criteria.

## The steps

1. **Name the kind and the job.** The product kind (`ui-product-kinds`) and one line: who arrives, what they came to
   do, what done looks like. Everything below is judged against that line.
2. **Glance, then squint.** Three seconds per width: where does the eye go first, second, third — is it the job's
   first step? Does it breathe, on one rhythm? Beside the references, is it in their tier?
3. **Walk the job** as that person, from arrival to done:
   - the path leads: one primary control finishes it, and nothing off the path competes;
   - every decision has its information beside it;
   - nothing the job needs is missing;
   - every label and value is true — a control does what it says, the same value reads the same everywhere, a total
     matches its lines;
   - every state the screen can enter still lets the job be done, or says why not;
   - the platform's minimums hold: target size, smallest text, contrast.
4. **Run the checklists**: the taste checklist and the quality bar (`design-references`), the kind checklist below,
   then the review checklist of each skill whose part the screen has — `ui-foundations` always, `ui-layout-and-states`
   always, `ui-data-display` for tables, tiles, charts and filters, `ui-forms` for any input, `ui-mobile` for phones,
   `ui-motion` where something moves, `ui-accessibility` always, `ui-writing` for the copy, `ui-polish` before hand-in.
5. **Write the findings**, worst first, one line each, tied to a picture and a region:
   `[blocker|polish] <picture> · <region> — what is wrong — what to do — <the rule it breaks>`.
   Name the region (top bar, second KPI tile, the form at 360), never "the page". No quality adjective without the
   thing behind it: not "cluttered" but "four borders around the filter bar".
6. **Plan the fixes**: one change per problem, at most five per pass, ranked by how much of the job each unblocks.
   Change the least that answers each; a new element copies its nearest sibling.
7. **Re-render and run every check again** — a fix can undo a pass. Three rounds at most; keep the best round, not the
   last, and compare each beside the one before.

## Severity

- **Blocker** (S1/S2 on design QA): the job can't be done, data or function is wrong, layout breaks at a major width,
  a WCAG 2.2 A/AA failure, a clear difference from the spec, or below the references' tier for a designer's own work.
- **Polish** (S3/S4): rhythm, alignment, density, hierarchy, detail — always written down, never a reason by itself to
  send an engineer's work back.

## The kind checklist

The critic, the second look and design QA hold every screen to this too, in these words:

First name the product kind from the brief and the pictures — a landing page, a web product or a mobile app — and
hold the screen to that kind's bar. Every kind: the first thing the eye lands on is what matters most; every state
shown is designed, with real copy and a next step; focus is visible; text contrast is at least 4.5:1 and meaningful UI
parts 3:1; nothing is told by colour alone; targets are at least 24px, and 44pt or 48dp on touch. A landing page: the
first screenful says what it is, for whom and what to do next, with one call to action and real evidence of the
product; each section makes one point; motion never holds up reading. A web product: density fit for daily use,
numbers right-aligned in tabular figures, filters and their results in view together, no marketing devices (display
type, gradient panels, illustrations) inside the tool. A mobile app: the platform's navigation and controls, the
primary action within thumb reach, safe areas and the keyboard respected, the platform's text sizes.

## Review checklist

- [ ] Kind and job named; judged at the kind's primary widths, light and dark, every state.
- [ ] The three-second glance lands on the job's first step; beside the references, same tier.
- [ ] The job walk passes all six checks.
- [ ] Taste checklist, kind checklist and each relevant skill checklist run.
- [ ] Findings worst first, each tied to a picture, a region and a rule; no bare adjectives.
- [ ] At most five fixes per pass; every check run again after fixing; best round kept.

Sources: the job walk, the five-moves cap and the "nearest sibling" rules adapt Mobbin's `redesign-screen` skill
(MIT, github.com/mobbin/skills); the full list of sources and licences is in `references/sources.md`.
