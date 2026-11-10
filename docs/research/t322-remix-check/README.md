# T-322 — the designer's remix flow, end to end (2026-11-03)

`equipe-design` main (`cbc2621`) linked onto a PATH folder; its `mcp` server
driven over stdio as the daemon's config names it (`"design": {"type":"stdio",
"command":"…/equipe-design","args":["mcp"]}`). Script: `/tmp/t322-run/drive.mjs`
(not kept).

| Step | Result |
|---|---|
| `tools/list` | `design_emit, design_extract, design_library_*, design_remix, design_report` |
| `design_extract` `https://equipe.im/` | ok — **fake engine** (schema example, not measured): the real engine isn't wired to MCP yet (T-323) |
| `design_report` | 6 limits (single page, unused colours, states not on page, …) |
| `design_remix` brand `#0f766e` "Tidewater", 2 variants | a, b with sheets (`remix-a-sheet.png`, `remix-b-sheet.png`), `contrastFails: 0` |
| `design_emit` a, `dtcgLegacy: true` | `tokens.css` + `dtcg/{primitive,semantic,semantic.color.light,semantic.color.dark,component}.json` |
| `equipe design check` on that folder | **fails: no `tokens.json`** — the legacy layout is equipe's own `design/tokens/`, which `design/scripts/build.mjs` turns into `tokens.json`; a project without that build has none |
| same, after equipe's `npm run build` on the emitted files | tokens 299 read, layers, dark ✓; light contrast ✗ `chip.selected-fg 4.27`, `badge.{ok,work,wait,err}-fg 4.09–4.33` (b: same, 4.08–4.34) — real misses the remix's own count doesn't measure |

Found on the way, fixed here: `equipe design check` failed equipe's own
`design/` — it read translucent tints (`color.raw.alpha.ink-5`) as text, and
measured `button.primary-fg` on the page instead of `button.primary-bg`. Now
equipe's `design/` passes (0 to fix).

Left for equipe-design: `design_emit` writes the `tokens.json` equipe reads;
the remix measures component text on its own fill (badges, chips); the real
engine behind `design_extract` (T-323).
