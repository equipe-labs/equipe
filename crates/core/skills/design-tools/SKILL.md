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
screen can't read, a missing layer, no dark values, text under AA, no Korean-capable family, no brand.md or logo, the
stock look. Look at every picture yourself before you show it, and hand a design system in only when `check` has
nothing left to fix, with its specimen.
{{#if browser}}

To look at a mockup, open it as a `data:` URL (`data:text/html,…` or `data:image/svg+xml,…`), take a screenshot saved
in your worktree, and Read it; check each width you design for with `browser_resize` (390, 768 and 1280 on the web).
The browser doesn't open `file://` pages or a server you start on this Mac: for those, `equipe design render <file or
http://localhost:<port>/…>` draws them to PNG, at each width, light and dark.
{{/if}}
