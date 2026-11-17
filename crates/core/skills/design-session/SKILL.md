---
name: design-session
description: Use when the person starts a design session with you (a message opening with `<design-session>`) — live, short rounds on one page — or asks you to prepare the taste tuner's set.
---

# Design sessions

A design session is the person and you on one page or screen, together, in rounds of minutes — not a solo build
handed in hours later. The point is to learn quickly the style they actually want. The kickoff names the page, the
project and whether their style profile is thin.

## Each turn

- **The first turn is quick.** Read the style profile at the top of the project's `design/taste.md` and its notes,
  then pick 3–6 references for the page's patterns (`design-references`; reuse what `design/references/` has) and make
  one rough take — rough is right here. Show it live and ask one question. Minutes, not an hour.
- **Small steps.** Each later turn changes one or two things the person asked for, not the whole page.
- **Say why, with the trade-off.** Every take, option and reference you show carries one plain line on why, one or
  two pros and one or two cons — in the person's language, short — so they see the reasoning, not only the
  picture. On a decision card, that is each option's `description` and its `changes`.
- **Every turn ends with something visible and one focused question** — a single taste call, with your
  recommendation: "Tighter or airier hero?", never a list of questions.
- **Never go quiet.** A step that would take more than a few minutes is split, and you show where it stands before
  going on.
- **Show it live.** Build the page as HTML and CSS from the project's tokens in `design/sessions/<date>-<page>/` (only
  `design/` is yours to change, as always) and run `equipe design session show <file.html | http://localhost:port/path
  | render.png>` after each change: the page opens beside the conversation on the person's screen, at 1440 wide with a
  390 toggle, and reloads each time you show it. A dev server you start in your worktree works too: show its address,
  and keep it running until the person is done (equipe tells you while they have it open in the live preview).
- **The critic still comes first.** Before a turn shows a take as a round's result, it goes once to `equipe design
  critique <render> --goal "…"` (a quick pass, not three rounds); say in a line what it found and what you changed.
  Never grade your own work and never call a take good — ask the person.
- **Pins.** The person's message may open with `<design-pins>`: each pin is a spot on the page (CSS pixels at the
  width given, `document.elementFromPoint` finds the element) with their note, its crop attached. Address each, then
  mark it: `equipe design session done <n> "<what changed, one line>"` — their screen ticks the pin.
- **Model.** Rounds run on a fast model; when the person asks to polish, the stronger model takes over: then do the
  full `design-process` pass on what was chosen — every state, light and dark, the widths, the critic's rounds.

## Taste, after every round

Taste calls are the person's: you offer, they decide. After each round, write down what it taught you:

- The style profile, at the top of `design/taste.md`: `equipe design session profile --density "…" --tone "…" --type
  "…" --color "…" --liked "a; b" --disliked "c"` — only the fields this round changed, in a few words each, from what
  the person said and chose. Its `Leans` line is equipe's (from the taste tuner and their reactions): leave it as it is.
- A pattern they showed twice becomes a note, unconfirmed, as `design-process` says; their thumbs and "more / less like
  this" are recorded by equipe already.

Follow the profile in every later turn; where a take departs from it, say which line and why.

## The taste tuner's set

When asked to prepare the taste tuner, put 24–40 real product screens from `design/references/` and your finds in
`design/tuner/set.json`: `{"items": [{"picture": "references/<pattern>/<file>.png", "source": "<product> — <screen>",
"axes": {"density": -60, "contrast": 20}}]}`. Place each on the axes it shows, -100 to 100: `density` (airy → dense),
`contrast` (light → dark), `color` (restrained → colorful), `type_scale` (small → large type), `ornament` (plain →
ornate), `layout` (grid → freeform). Balance it: for every axis some pictures at each end, and many pairs that differ on
one or two axes and hold the rest. Run `equipe design tuner check` until it says the set is ready. Never anyone's
private screens — public pages only, as `design-references` says.
