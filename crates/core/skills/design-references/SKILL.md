---
name: design-references
description: Use for a brand, a rebrand or a new visual direction — the reference board, directions pictured as palettes and type pairs, the person's pick — and, on a build, for one tool when a need appears.
---

# References and taste

A strong designer studies the best work before drawing anything, and takes from it what can be learned — patterns,
layouts, rhythm, type, colour — never what belongs to someone else. The rule, without exception: copy patterns and
layouts as they are if they serve the brief, but never another company's logo, name or distinctive brand marks, and
never its words; every line of copy is the product's own, from its brief. A reference's "what to learn" line that
says to take a brand or its words is refused by the tools; say what its structure teaches instead.

## When

- **A brand, a rebrand or a new visual direction** — the ticket asks for one, or the person is to choose a look: the
  full pass below, every step.
- **A build** — a page or screen from a clear brief, in a design system that exists: work as you always do; nothing
  here is a step you owe. Reach for one tool when a concrete need shows, and only for it, in a few minutes: a colour
  the tokens don't have → `palette` on it, light and dark, before it ships; a pattern you haven't designed before (a
  pricing table, an onboarding, a comparison) → one or two references of it with `refs shoot`; a new typeface, or a
  script the type doesn't set → `type` on it. Never as a ritual, and never in place of finish.

The colours that ship meet AA, light and dark — the text on its background, a button's label on its fill; a pair
under it is fixed before anything is handed in.
{{#if board_tools}}

Run these through your board tool `design_tools`, its `args` the words after `equipe design` (`["refs", "shoot",
"<url>", "--learn", "…", "--aspect", "layout"]`): equipe runs them for you outside your sandbox, in your worktree,
reading only the listed reference sites — nothing to ask. One call at a time; several pages go in one `refs shoot`.
{{/if}}

From a terminal the same commands are `equipe design …`; they read the web and draw in Chrome, so outside an
engine's sandbox only.

## The full pass

Say which step you are on{{tools_at}}:

1. **The brief.** Who it is for, what it must do, what is locked (the brand's `## Locked` lines) and what is free, in
   `brand.md` (the `design-process` skill has the binding brief). Name the kind of page and its sections before you
   look for anything.
2. **The board.** Collect at least five references from at least two sources, each with one line of what to learn
   and its aspect (`pattern`, `layout`, `rhythm`, `type`, `colour`). `equipe design refs sources` lists the sources,
   how each is read and whether it can be used now; `equipe design refs search <source> <word>` says where to look
   (a category is a word — `hero`, `pricing` — and for 21st.dev it lists the components); `equipe design refs shoot
   <url>… --learn "…" --aspect …` shoots public pages, up to eight a call, paced, as robots.txt allows. Lapa Ninja
   (landing pages) and Mobbin (real product screens) are browsed in your own browser: open the page, save a
   screenshot in your worktree, and record it with `equipe design refs add <picture> --url <page> --learn "…"
   --aspect …`. Mobbin needs the person's own sign-in and paid plan: use it only when `refs sources` says the squad's
   browser is signed in; otherwise skip it and say so — never sign up, sign in, accept terms or enter a key yourself.
   A page that asks whether a person is there is never answered by you or a tool: hand it to the person with
   `show_my_browser`, or pick another page. Then `equipe design refs board --brief "<the brief in a line>"` draws the
   contact sheet (`design/references/board.png`) and says what the board still needs; look at it yourself.
3. **Directions, as pictures.** Draw three directions from the board, as `design-process` asks, each a layout
   pattern, a palette and a type pair, and each citing the references it learns from: `equipe design direction
   propose <name> … --ref R2 --ref R5`. Try the palettes in one call on the page itself with `equipe design palette
   "<Name>=<text>-<bg>-<primary>-<secondary>-<accent>" … --fonts "<Heading>/<Body>" --page <your page or mockup>` —
   your page in each palette, light and dark (a page of our own where there is none yet), the WCAG AA contrast of
   every text pair, and tokens in the design system's format in `design/palettes/`; a palette with a pair under AA is
   fixed before it is offered. Try the type pairs in one call with `equipe design type "<Heading>/<Body>" …
   --locales <the product's languages>`: a specimen at real sizes in each language, what each family lacks of a
   script, its licence and what it weighs to load (`design/type/specimen.png`); a family that lacks the product's
   script is paired with one that has it. Mock each direction's first screen up in its tokens, so each option is
   seen as it would live.
4. **The person picks.** After the critic has seen them, put the directions to the person in one `ask_decision`
   question: each option with its pictures (the mockup, its palette and type sheets) and what it takes from which
   reference, your recommendation marked and why, one bolder than your recommendation said as such. Record their
   pick with `equipe design direction choose`.
5. **Build, then the gates.** Build the chosen direction in its checked tokens, and run the existing checks: `equipe
   design lint --render`, the critic, the second look, and `kept check` for a redesign.

## Components

A component from 21st.dev or a public shadcn registry can be a starting point for a pattern: `equipe design
component <its page, or the npx shadcn add line>` pulls what is public into `design/vendor/<name>/` with a
`SOURCE.md` (where it came from, its licence, what it needs, what to restyle). Where a registry wants a key or its
site closes it to tools, only its page and licence are recorded: rebuild the pattern from our tokens instead. Restyle
every colour, font, radius and picture it brings to our tokens before it goes into the product, keep its licence's
notice, and remove the vendor copy before you hand in.
