---
name: working-with-design
description: Use when you write a ticket for the designer or one that changes a screen, start a project with a designer, the person turns a design down, brings a UI map screen, or asks to see a flow as a video or references.
---

# Working with design

A design ticket says the problem, who it is for, the constraints and how success shows — never the layout, the
composition or the visual answer: those are the designer's, and it brings you options to put to the person. On a new
project with a designer, ask for its **starter brand** first: one designer ticket titled "Starter brand", and nothing
more — the platform's tools make the palette, type, logo, a photograph, tokens and a brand sheet, and the designer takes one
art-direction turn. Never open the old five-step setup by yourself. The **deep brand system** (the full brief, component sheets,
guidelines and UI map, a ticket a step titled "Deep brand system: <step>") starts only when the person chose it on the
Brand page, which shows its cost, or said yes to you. You may suggest it once in a project's life, in one plain line
with its cost from the Brand page ("about $X"), and only when the signals justify it: many screens (eight or more
built or planned), several people contributing, or the person asking for a consistent look across a growing product;
never again after a no or silence. It is the person's paid choice, not an approval of internal work. An imported site (`reference/` in its repository) keeps its original design: it gets the designer's
light setup, one ticket that only reads the brand, and never a ticket to apply a design system, restyle or reword pages
the person didn't ask to change. A ticket that changes an imported site's look quotes the person's request; equipe refuses
a live deploy that restyles one without it. When an engineer's ticket changes a screen, its acceptance criteria include the UI map
brought up to date: the engineer runs `equipe design flows shoot`, or a designer's ticket follows it. A screen
change the person asked for is one slice, on one ticket: the designer prototypes or specs it on the ticket's branch and
passes the baton (`pass_baton`) to the frontend, who builds it in the same worktree — never a second ticket for the build,
nor a design QA ticket after it: equipe's precheck checks the built screens against the prototype, and a designer looks
only at what it fails. When the person
brings you a screen from the map ("Onboarding › Sign in › Error") or asks to see a flow as a video, it is the
designer's: route it to them with the flow, screen and state named; scope and priority stay yours.
When a project the person builds a UI on has no design system of its own yet — no tokens, no component registry —
offer one before its screens multiply: the designer's tickets in order (the brand, the tokens, a registry with every
component and a docs page each), then frontend's screens built only from it; the person decides whether and when.
A new product's first sprint includes its starter brand (`brand-kit`: logo candidates that passed the quality gate, the
designer's pick, the kit, palette and type as tokens, mostly made by tools) before its screens are polished; the designer shows the pick with
why and never asks the person to choose first; the person can switch on the Brand page. Every ticket that plans or changes a screen carries `Design spec: design/screens/<slug>.md`
(or says the designer writes it first) and acceptance that includes design acceptance; your `design-loop` skill says how
a spec is reviewed before build, accepted after it, and how a number from the growth run or the sprint review becomes a
design ticket with its evidence and a hypothesis. A new web UI with no stack yet is built on the default one
(`web-stack`: React, Tailwind, shadcn/ui, lucide), never a stack the engineer improvises; a project that has a stack keeps it.
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
Anything the person should look at or try goes in front of them (a card's Try it, `equipe design session show
<url|file>`, or `show_my_browser`), never only into a member's own browser; name what to click as they see it.
A page you serve on your machine (`localhost`) opens in their live preview, big and fully usable: keep its server
running until the person is done — equipe tells you while they have it open and when they close it.
