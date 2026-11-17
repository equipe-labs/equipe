---
name: working-with-design
description: Use when you write a ticket for the designer or one that changes a screen, start a project with a designer, the person turns a design down, brings a UI map screen, or asks to see a flow as a video or references.
---

# Working with design

A design ticket says the problem, who it is for, the constraints and how success shows — never the layout, the
composition or the visual answer: those are the designer's, and it brings you options to put to the person. On a new
project with a designer, ask first for its setup — brief, logo, design system, UI map, design QA — as tickets, a step
each or the first two together. When an engineer's ticket changes a screen, its acceptance criteria include the UI map
brought up to date: the engineer runs `equipe design flows shoot`, or a designer's ticket follows it. When the person
brings you a screen from the map ("Onboarding › Sign in › Error") or asks to see a flow as a video, it is the
designer's: route it to them with the flow, screen and state named; scope and priority stay yours.
When a project the person builds a UI on has no design system of its own yet — no tokens, no component registry —
offer one before its screens multiply: the designer's tickets in order (the brand, the tokens, a registry with every
component and a docs page each), then frontend's screens built only from it; the person decides whether and when.
That setup brief is the design's — `design/brand.md`, in the project's repository with its system, which may be
public: the product's look and voice only. The person's own plans, the people they name and their numbers stay in
your brief and plan, in the squad's folder.

## A design session first

Designs that arrive late, all at once, mostly disappoint. Propose a short design session (`propose_design_session`,
one line on why) rather than a long solo build when a new screen, a redesign or a visual direction is about to go to
the designer; when the person turned a design down, sent it back or asked for changes twice; when the project's style
profile (the top of its `design/taste.md`) is missing or thin; or when a designer ticket is about to explore several
directions. equipe puts it on a card at most once per ticket or screen, never in their quiet hours, and offers to tune
their taste first when the profile is thin. On "Not now" or "Let the designer go ahead", carry on as planned, without
asking again. When the person asks for a session themselves, call it with `asked`: it starts at once.

## Showing references

When the person asks to see references, components or screens to work from ("show me components I could use"), show
them in the same turn: search the design libraries you have (21st.dev and Mobbin, read-only for you; else public
component pages such as the shadcn registry) and call `show_references` with the best 4 to 8, each with its picture,
source link and a line on why it fits. Never describe them in words instead or send the person to a site. Mobbin's
pictures are private study material: shown in equipe, never shipped or committed. Building on them is the designer's.
