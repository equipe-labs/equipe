---
name: design-tools
description: Use before you run `equipe design …` or look at a mockup you made — what init, audit, specimen, render, shoot, diff and check do.
---

# Your design tools

Your tools, run in your worktree{{tools_at}}: `equipe design init` lays out a `design/` folder to fill in (brand.md,
layered tokens.json, guidelines.md); `audit <url>` harvests a brand from its site as Chrome renders it — colours by
the area they cover and where (the logo's own from its file), type families and webfonts, the type scale, radii,
shadows, the logo files and its voice — into `design/audit/` with one sheet; `specimen [--tokens <file>] --png` draws
a system from its tokens alone (two real screens light and dark, palette with contrast, type, spacing, radius,
shadow); `render <file> --widths 390,1280 --dark` draws any HTML or SVG mockup to PNG; `shoot <url>…` shoots running
pages (`http://localhost:<port>/…` included) at 390, 768 and 1280, light and dark; `diff <spec.png> <built.png>` puts
a mockup and what was built side by side and lights up where they differ; `check` says what is wrong — tokens equipe's
screen can't read, a missing layer, no dark values, text under AA, no family for the product's script, no brand.md or logo, the
stock look. `lint <files or URL> --render
--screen <flow/screen>` holds a mockup or a page to the design system in code at seven widths — the floor, the habits
of generated screens, finishing notes (`--rules` lists every rule, `--tier floor` the floor alone, `--waive <id> --why
"…"` keeps a habit with its reason) — and keeps the check and its pictures in `design/checks/`; `second-look
<flow/screen>` has fresh eyes judge those pictures against the brief and the system; `direction` keeps the log of
directions proposed and chosen; `kept record` and `kept check` make sure a redesign loses no link, field, action or
heading. For taste (the `design-references` skill has when and how): `refs` keeps a brief's reference board in
`design/references/` — `sources`, `search`, `shoot` public pages with what to learn from each, `add` a screenshot
from your own browser, `board` draws the contact sheet; `palette` tries two to four palettes on a real layout, light
and dark, with their contrast, as tokens; `type` sets type pairs as a specimen in the product's languages, with
coverage, licence and weight; `component` pulls a public component's source in as a starting point, with its source
and licence, to restyle to the tokens. Look at every picture yourself before you show it, and hand a design system in only when `check` has
nothing left to fix, with its specimen.
{{#if browser}}

To look at a mockup, open it as a `data:` URL (`data:text/html,…` or `data:image/svg+xml,…`), take a screenshot saved
in your worktree, and Read it; check each width you design for with `browser_resize` (390, 768 and 1280 on the web).
The browser doesn't open `file://` pages or a server you start on this Mac: for those, `equipe design render <file or
http://localhost:<port>/…>` draws them to PNG, at each width, light and dark.
{{/if}}
