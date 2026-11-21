---
name: design-loop
description: Use when a ticket plans, designs, builds, accepts or measures a screen — the spec file, the design review before build, design acceptance, and design changes proposed from the numbers.
---

# Plan, design, build, measure, improve

A product company's loop, run on files the whole team can open. The repository is the source of truth; a Figma-style
tool (Penpot) mirrors it, never the other way round.

## The screen spec

Every screen that matters has `design/screens/<slug>.md` in the product's repository, written by the designer:

```
---
title: Pricing
status: draft | in-review | approved | built | verified
ticket: T-123
flow: Onboarding
route: /pricing        where the built screen is, for the screenshots
updated: 2026-11-21
---
## Job            what the person comes to do, and what they should feel
## Layout         hierarchy and sections, in terms of the system's components and tokens
## States         empty, loading, error, success, long text, offline
## Copy           the words, in the brand voice
## Behaviour      interactions, keyboard, motion
## Success metric the event or funnel step this screen should move, and from what to what
## History        what shipped when, and what the numbers said
```

Pictures sit beside it: `design/screens/<slug>/design.png` (the design), `built-1280.png` and `built-390.png` (the
built screen; in a scaffolded project `npm run build && npm run shots` writes them from each spec's `route`). The team's Design view reads these files, so a spec without them shows as unbuilt.

## Plan

The PM's ticket for a screen carries `Design spec: design/screens/<slug>.md` (or "designer writes it first") and
acceptance criteria that include design acceptance: "the built screen shows every state in the spec, uses only the
system's tokens and components, and its screenshots at 390 and 1280 are in `design/screens/<slug>/`". A plan's design
notes name the screens by their spec path.

## Design review before build

The designer's spec and picture go to a fresh critic first (`design-process`), then to the person when a taste call
is in it, and the status moves to `approved`. Small changes inside an approved system need only the PM's check against
the brief. The frontend engineer starts from an approved spec, in the same worktree, by baton, never from a
description in chat.

## Build and design acceptance

The engineer builds from the spec with the system's components, saves the built screenshots where the spec expects
them and runs the design precheck. The designer (or, on a team without one, the PM against the spec's acceptance
list) compares built with design, state by state, and moves the status to `verified` with what differs, if anything.
A build ticket is not done until its screen is `verified`; a difference goes back as a fix on the same ticket.

## Measure and improve

Every spec names its success metric. The growth run and the sprint review put numbers beside it; this loop does not
count anything itself. When the numbers point at a screen — a funnel step that drops, a page nobody scrolls, an
experiment's losing variant — the PM opens one design ticket for it with the evidence (the same E-numbers), the spec
link and a hypothesis in one line: "if <change>, then <metric> moves from <a> to <b>, read after <n> days". The
designer proposes the change as an updated spec (and an experiment, when the platform has them), it goes through the
same review, build and acceptance, and when the measurement comes in, the result goes into the spec's `History`
(worked, no change, worse) — a worse change is reverted, not defended. Never redesign a screen on taste alone when
the numbers already say what is wrong, and never on numbers alone when they are too thin to say.

## A design file, when there is one

If the project has `design/penpot.json` (a Penpot file link), keep that file in step: tokens pushed from
the tokens file in `design/`, a frame per spec with its status, the file's link on the spec. The files in the repository win
when the two differ; say so in the hand-in rather than editing the mirror and forgetting the repository.
