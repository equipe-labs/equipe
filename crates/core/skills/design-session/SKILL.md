---
name: design-session
description: Use when the person starts a design session with you (a message opening with `<design-session>`) — live, short rounds on one page — or asks you to prepare the taste tuner's set.
---

# Design sessions

A design session is the person and you on one page or screen, together, in rounds of minutes — not a solo build
handed in hours later. The point is to learn quickly the style they actually want. The kickoff names the page, the
project and whether their style profile is thin.

## Prototype mode

The person wants to see the design applied, look at it, touch it, give feedback and iterate — and only then have it
built in the real code. So a session works on a prototype they can use, not on pictures of one:

- **References first.** Before proposing any direction — at the start, and whenever the person asks for new
  directions — pull 3–5 references for the page's patterns from the design MCPs you have connected (21st.dev, Mobbin;
  else the web, as `design-references` says) and put them on screen with `show_references`, each with why, what to
  take and what to leave. Don't wait to be asked. Keep the pull small — ask each MCP for a handful of results — since
  every picture a search returns stays in your conversation and fills it fast.
- **A runnable prototype.** Build the page in the session's folder, `design/sessions/<date>-<page>/` (only `design/` is
  yours to change, as always): HTML and CSS from the project's tokens, or a route of the dev server you run in your
  worktree. Show it with `equipe design session show <file.html | http://localhost:port/path>`: it opens live beside
  the conversation on the person's screen and reloads each time you show it. Keep the server running until the person
  is done (equipe tells you while they have it open).
- **Variants side by side.** When there are real alternatives, make two or three — `index-a.html`, `index-b.html`… —
  and show each with `--variant A`, `--variant B`, `--variant C`: the person sees them next to each other, live, and
  picks one with "Use this".
- **A change log.** Keep `CHANGELOG.md` in the session's folder: one line per round — what changed and which feedback
  or pin it answers. It becomes part of the implementation ticket's spec.

## Each turn

- **The first turn is quick.** Read the style profile at the top of the project's `design/taste.md` and its notes,
  show the references, then make one rough take — rough is right here. Show it live and ask one question. Minutes, not
  an hour.
- **Short replies, fast rounds.** Each later turn changes one or two things the person asked for, not the whole page.
  It applies them to the prototype within a couple of minutes, shows it again, and answers in a few lines.
- **No heavy checks in a round.** No width × theme × text-size matrix, no axe or contrast runs, no screenshot sets, no
  design QA, no QA tickets, and no new ticket per feedback item: those make every round take ten minutes. A glance at
  the one width the person looks at is enough.
- **Say why, with the trade-off.** Every take, option and reference you show carries one plain line on why, one or
  two pros and one or two cons — in the person's language, short — so they see the reasoning, not only the picture.
- **Every turn ends with something visible and one focused question** — a single taste call, with your
  recommendation: "Tighter or airier hero?", never a list of questions.
- **Never go quiet.** A step that would take more than a few minutes is split, and you show where it stands before
  going on. The person's messages come first: equipe stops a long command of yours so you read them at once.
- **Pins.** The person's message may open with `<design-pins>`: each pin is a spot on the page (CSS pixels at the
  width given) with their note, the element there (`element:` its selector, `text:` its words, `box:` where it sits)
  and its crop attached. Address each, then mark it: `equipe design session done <n> "<what changed, one line>"` —
  their screen ticks the pin.
- **Grading.** Never grade your own work and never call a take good — ask the person.
- **Model.** Rounds run on a fast model; when the person asks to polish, the stronger model takes over for the chosen
  take.

## Use this: from prototype to code

When the person picks a variant with "Use this", equipe opens the implementation ticket with the prototype as its spec
(or updates the one it opened before) and tells you. That ends the session's rounds on the page: make sure the
prototype and its `CHANGELOG.md` are saved, then the ticket's work runs as usual — the full `design-process` pass,
every state, light and dark, the widths, larger text, the critic's rounds and design QA.

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
