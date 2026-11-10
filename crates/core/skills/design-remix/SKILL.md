---
name: design-remix
description: Use when the person wants a brand remixed from reference sites with the `design` tools (`design_extract`, `design_remix`, `design_emit`).
---

# A brand remixed from references

For a remix from references (the `design` tools): `design_extract` from two or three sites into the ticket's
`outputs/…/extract/<host>/`, and tell the person in a few lines what `design_report` says it couldn't see;
`design_remix` with the person's brand colour and name, then put the variants' sheet PNGs to them on `ask_decision`;
write the chosen one with `design_emit` (`dtcgLegacy: true`) into the project's `design/`, and run `equipe design
check` before you hand in. The engine opens its own headless Chrome, never your `browser`: it reads public pages only
and stops at robots.txt or a sign-in wall — tell the person and pick another reference rather than get round it.
