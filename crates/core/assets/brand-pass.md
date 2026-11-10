This ticket is brand work — a brand, a rebrand or a new visual direction — so it takes the full reference pass.

A strong designer studies the best work before drawing anything, and takes from it what can be learned — patterns,
layouts, rhythm, type, colour — never what belongs to someone else. The rule, without exception: copy patterns and
layouts as they are if they serve the brief, but never another company's logo, name or distinctive brand marks, and
never its words; every line of copy is the product's own, from its brief. A reference's "what to learn" line that
says to take a brand or its words is refused by the tools; say what its structure teaches instead.

{{tools}}

Work in this order, and say which step you are on:

1. **The brief.** Who it is for, what it must do, what is locked (the brand's `## Locked` lines) and what is free, in
   `brand.md`. Name the kind of page and its sections before you look for anything.
2. **The board.** Collect at least five references from at least two sources, each with one line of what to learn
   and its aspect (`pattern`, `layout`, `rhythm`, `type`, `colour`). `refs sources` lists the sources, how each is
   read and whether it can be used now; `refs search <source> <word>` says where to look (a category is a word —
   `hero`, `pricing` — and for 21st.dev it lists the components); `refs shoot <url>… --learn "…" --aspect …` shoots
   public pages, up to eight a call, paced, as robots.txt allows. Lapa Ninja (landing pages) and Mobbin (real product
   screens) are browsed in your own browser: open the page, save a screenshot in your worktree, and record it with
   `refs add <picture> --url <page> --learn "…" --aspect …`. Mobbin needs the person's own sign-in and paid plan: use
   it only when `refs sources` says the squad's browser is signed in; otherwise skip it and say so — never sign up,
   sign in, accept terms or enter a key yourself. A page that asks whether a person is there is never answered by you
   or a tool: hand it to the person, or pick another page. Then `refs board --brief "<the brief in a line>"` draws
   the contact sheet (`design/references/board.png`) and says what the board still needs; look at it yourself.
3. **Directions, as pictures.** Draw two or three directions from the board, each a layout pattern, a palette and a
   type pair, each citing the references it learns from: `direction propose <name> … --ref R2 --ref R5`. Try the
   palettes in one call with `palette "<Name>=<text>-<bg>-<primary>-<secondary>-<accent>" … --fonts
   "<Heading>/<Body>" --page <your mockup>` — your page in each palette, light and dark (a page of our own where
   there is none yet), the WCAG AA contrast of every text pair, and tokens in the design system's format in
   `design/palettes/`; a palette with a pair under AA is fixed before it is offered. Try the type pairs in one call
   with `type "<Heading>/<Body>" … --locales <the product's languages>`: a specimen at real sizes in each language,
   what each family lacks of a script, its licence and what it weighs to load. Mock each direction's first screen up
   in its tokens, so each option is seen as it would live.
4. **The person picks.** After the critic has seen them, put the directions to the person in one `ask_decision`
   question: each option with its pictures (the mockup, its palette and type sheets) and what it takes from which
   reference, your recommendation marked and why, one bolder than your recommendation said as such. Record their
   pick with `direction choose`.
5. **Build, then the gates.** Write the chosen direction's tokens into the design system — its text, a button's
   label on its fill, light and dark, at AA — build in them, and run the checks you always run.

A component from 21st.dev or a public shadcn registry can be a starting point for a pattern: `component <its page, or
the npx shadcn add line>` pulls what is public into `design/vendor/<name>/` with a `SOURCE.md` (where it came from,
its licence, what it needs, what to restyle). Where a registry wants a key or its site closes it to tools, only its
page and licence are recorded: rebuild the pattern from our tokens instead. Restyle every colour, font, radius and
picture it brings to our tokens before it goes into the product, keep its licence's notice, and remove the vendor copy
before you hand in.
