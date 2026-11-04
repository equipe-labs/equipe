---
name: ui-floor
description: Use when you change a screen's code — the design floor equipe checks after each edit, and how to check a screen before you hand it in.
---

# The design floor for what you build

Screens are held to the project's design system in code, as you write them. After each edit to a UI file (HTML,
CSS, TSX, JSX, Vue, Svelte) the floor is read in what you just wrote, and what fails comes back to you by itself
(where it doesn't, run `equipe design lint <file> --tier floor`{{tools_at}} after each edit): colour, type and spacing
only from the design system's tokens, focus that shows, a reduced-motion version of whatever moves, every control
named, no invented figures, customers or quotes. Fix it in the same edit; the floor is never waived. A finding in
code you didn't touch is not yours to widen the ticket for: say it in your hand-in, and propose a ticket.

Before you hand in a change to what users see, check the changed screens drawn as well: `equipe design lint
<changed files> --render --url http://localhost:<port>/… --screen <flow/screen>` draws each at 360, 390, 768, 1024,
1280, 1440 and 1920 and adds what only a drawing shows — nothing running off the side, no clipped text, contrast, the
`data-primary` action in the first screenful. Hand in with the floor passing on what you changed. Where the
designer's spec keeps one of the habits it reports (a centred layout, say), its reason is in `design/decisions/`
and the lint says so; anything else it reports as a habit, fix or name in your hand-in, and its notes are polish.
