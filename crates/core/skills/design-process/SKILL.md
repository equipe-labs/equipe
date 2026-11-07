---
name: design-process
description: Use for design work that brings options to the person — a new flow or screen, a direction, a revision — the steps, the references, the critic, the person's taste and recording decisions.
---

# How design work goes

Work in this order and say which step you are on: define the problem and how success shows; collect references — from
the squad's reference library first, then new ones, each with its link and what you take from it; for a new flow or
screen, its information architecture and the flow step by step, then low-fidelity wireframes, before any visual;
explore at least three genuinely different options (colour alone doesn't make another) and put them side by side in
one comparison sheet with your recommendation and why; take the person's critique; refine only the chosen option, at
full fidelity — every state it can have (the UI map's kinds, in the `ui-map` skill), light and dark, the widths it supports, real
copy; where it moves or flows across screens, a clickable HTML prototype, recorded step by step; hand off a spec an
engineer can build without guessing (token names, states, responsive rules, motion, accessibility, copy, asset paths);
after the engineer builds it, check the result in design QA.

Show design options as they would live: each applied to the product's real screens — the UI map's pictures
(`design/flows/`) shot again or mocked up with that option's tokens, or `equipe design specimen --tokens <file> --png`
when no screen exists yet — never as swatches alone.

Keep the squad's reference library in its folder, `design/references/`: a folder for each reference worth keeping —
its pages shot with `shoot`, its link, what to take from it and what to leave — read it before you search anew, and
add to it whatever you find; with the `design` tools, the design library (`design_library_search`) is part of it.
For a brand, a rebrand or a new visual direction, open the `design-references` skill before any option: a board of
five or more references from two or more sources, each with what to learn from it, then directions pictured with
their palette, type pair and layout; on a build, its tools are there for a concrete need, never as steps.
Before any option, write a short binding brief in `brand.md`: one specific reference (a named product, site or print
piece, and what to take from it — adjectives describe a region, a reference a point), what is locked and what is free,
and a one-line visual thesis.

Directions differ in structure — layout, type, imagery, density — not in colour alone. Never grade your own work, and
never show the person a picture the critic hasn't seen: every direction, refined screen and revision goes first to the
critic on the other engine: `equipe design critique <pictures…> --reference <the reference's shot> [--before <the
round before>] [--goal "…"]` sends them, with `brand.md`, to a second model that finds faults instead of praising, and
keeps its answer in `design/critiques/`. Only when it can't run, a sceptical subagent of your own takes its place, and
you say so. The rubric it judges by: First the whole: at a three-second glance and squinting, where does the eye go,
does it breathe — whitespace and rhythm — and next to the reference, does it look finished and expensive or like a
template? Then the details: hierarchy and density, alignment and spacing on the scale, type (scale, setting for the
product's script, numbers), colour roles and contrast, brand fidelity ("hide the logo: could this be another company?"), states and real
copy, and the stock looks — each finding tied to a region of the picture. Revise on its findings, at most three
rounds, each revision beside the one before (a fix that loses the whole is worse than the flaw it fixed), and keep the
best round, not the last; say in your hand-in what the critic found and what you changed.

The person's taste is a list
of notes, a line each — the squad's in its memory, `design taste notes`; a project's in its `design/taste.md`. Read
them before any proposal and follow them; where a proposal departs from one, say which and why. A note ending in
`confirmed` or written by the person outranks one still waiting for their yes; where they disagree, follow the
confirmed one. equipe also keeps, in `design taste`, every design question the person answers (what they chose, what
they passed over, their own words) and every piece of design work they send back, with why. When a pattern shows there
(the same pick or complaint twice), propose a new note: add it in one line, unconfirmed, naming the tickets it was
seen on — `- <note> _(noticed on T-<n>, <date>, seen on T-<a> T-<b>)_` — and leave the person's lines as they are;
they confirm it, correct it or remove it on the design screen. After each decision, record it in
`design/decisions/<date>-<question>.md` — the question, `Chose:`, `Picture:` (the chosen option's path), `Passed
over:`, `Why:` in the person's words, `Ticket:`.

Among your options, always one bolder than your recommendation, said
as such: a choice only teaches when there is one.

Record each direction you propose by its key choices, so one design doesn't drift into the last: `equipe design
direction propose <name> --layout "…" --type "…" --palette "…" --density "…" --motion "…" --ticket T-<n>`. It refuses
a direction that repeats one of the last three the person chose in every dimension the brand leaves free, or that
differs from its fellow options only in colour; change it until it is taken, never reword it to pass. When the person
chooses, `equipe design direction choose <name> --ticket T-<n>`.

Before you redesign a screen that exists, write down what it offers: `equipe design kept record <its source or URL>
--screen <flow/screen>`; after, `equipe design kept check <the new source or URL> --screen <flow/screen>` must find
nothing gone. A link, field, action or heading goes only where the brief or a decision says so, as `Remove <kind>:
<item>`.

Every screen you make is checked in code before anyone sees it: `equipe design lint <mockup or source> --render
--screen <flow/screen>` (`--url http://localhost:<port>/…` for one the app serves) holds it to the design system and
draws it at 360, 390, 768, 1024, 1280, 1440 and 1920. The floor is never waived: colour, type and spacing only from
the tokens, focus that shows, a still version of whatever moves, no invented figures, customers, quotes or logos,
every control named, nothing running off the side, no clipped text, text contrast, and the screen's primary action
— mark it `data-primary` — in the first screenful. The habits of generated screens (a label over every heading,
spaced capitals, one coloured word, a gradient first screen, cards in cards, emoji as icons, everything centred, one
radius and shadow everywhere, decorative dots, a form that pretends to send) are fixed, or kept with the person's or
the brief's reason: `equipe design lint --waive <id> --why "…"` writes it in `design/decisions/`. Finishing notes are
worth doing and never block. After each edit to a UI file the floor is read in what you just wrote and comes back
to you by itself; fix it before you go on (where it doesn't come back, run `equipe design lint <file> --tier floor`
after each edit).

The last critique before you hand in is a second look by fresh eyes: `equipe design second-look <flow/screen>
--criteria "<the ticket's acceptance criteria>"` gives a reviewer that saw none of your work only the pictures lint
drew at each width, the brief, the criteria and the design system, and keeps its verdict on the screen's record. Fix
every fix it lists, check again and ask again until it passes, or until three looks in a row have asked for fixes:
then hand in, and what is left goes to the person marked, the PM told. Never argue it into a pass, and never write
its verdict yourself. What it needs that the brief, the project's files and the design system don't give comes back as
open questions, not fixes: use the neutral wording it suggests, or ask the person with `ask_decision` while you carry
on — never stop the ticket on `block_on_person` for them. For a new screen, its first screenful is looked at alone first (`--first`), before you build the
rest on it. Only when no engine can run: start a subagent with nothing of your conversation, give it only what
`equipe design second-look <flow/screen> --prompt` prints, and record its answer with `--answer <file>`. A hand-in is
refused until every UI file you edited — not the directions you explored in `design/directions/` — was checked this
way, passed the floor and had a passing second look; say in it the floor's count, each habit kept and why, and what
the second look asked for and you changed.
{{#if board_tools}}

Give each design option its `images` and its `aspects` (what of it can go into another — type, colour, shape — in the
person's language): they answer in one click. On a mix (one option with an aspect of another), make the mixed option
and ask again with it beside the two it came from; on another round, bring a genuinely different round that takes
their why into account.
{{/if}}
