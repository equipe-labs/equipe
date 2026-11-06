# Pictures in the conversation (T-336) — diagram theme and HTML cards

The values and templates that make pictures in a conversation look like equipe's screens: the same colours·type·spacing. Every value comes from `tokens/` (read after `npm run build` from `build/css/tokens.css`·`scripts/lib.mjs`). The contract (when to use which picture, size·safety) is `docs/VISUALS.md` (T-335). This covers only the **look**.

```
visuals/
  mermaid/   theme.{light,dark}.json · classdefs.{light,dark}.mmd · theme.rs (generated) · build.mjs · examples/
  cards/     card.mjs (data → HTML) · card.css · examples/{board,compare,figure}.json
  preview/   PNGs of every template·diagram (light·dark × 360·720)
  proposals/ the three options before the person chose (A cards·B lanes·C pills) — A was chosen (taste.md)
  contrast.mjs → contrast.md   computes text 4.5:1 · graphics 3:1, exit code 1 if anything falls short
```
To regenerate: `node mermaid/build.mjs` · `node contrast.mjs`. Generated files (`theme.*`, `classdefs.*`, `theme.rs`, `contrast.md`) are never edited by hand.

## 1. Mermaid — the chosen look: A, cards

**Principles**: nodes are rounded cards (`color.surface` fill + a 1.5px `color.border-strong` line), lines and arrows `color.muted` 1.5px. A node with a status gets a tinted fill in the status colour + a border in the same colour + **a symbol before the label** (never colour alone). Text is always `color.text`. The role (backend and the like) is the label's second line (`<br/>`). One line is 14px, labels wrap at 30 characters at most, node spacing 28 · rank spacing 56 · padding 22×14.

| Status | Class | Symbol | Fill · border tokens |
|---|---|---|---|
| Next (default) | `next` (the same look when none is given) | ○ | `surface` · `border-strong` |
| In progress | `work` | ● | `work-bg` · `work` |
| Your turn | `wait` | ◆ | `wait-bg` · `wait` |
| Done | `ok` | ✓ | `ok-bg` · `ok` |
| Blocked | `err` | ✕ | `err-bg` · `err` |

How members write it: just the class name, as in `T336("● T-336 Chart theme<br/>designer"):::work`. **Never colour codes** (light and dark differ, and a member can't know which). Pie slices are painted in the order `accent → ok → work → wait → err → border-strong`, with slice text white in light and the `bg` colour in dark (the slice colours overlap the status colours, so for values that aren't statuses the legend's words carry the meaning).

### Applying it on the desktop (T-338 — `mermaid_svg` in `crates/desktop/src/render/raster.rs`)
1. **Copy** `design/visuals/mermaid/theme.rs` into `crates/desktop/src/render/` (a generated file — don't edit it; when values change, design makes it again. Add a line to `design/copies.json` for the copy-drift check).
2. `Theme`: replace today's hand-written colour block (lines 97–111) with `equipe_theme_dark()` / `equipe_theme_light()`. The font line (`"Helvetica Neue", "Apple SD Gothic Neo", sans-serif`) comes in it as is.
3. `RenderOptions { theme, layout: equipe_layout(LayoutConfig::default()) }`.
4. If the source has **no** `classDef`, draw it with `CLASSDEFS_DARK` / `CLASSDEFS_LIGHT` appended to the source's end after a newline (if it has some, the member styled it, so leave it). The source shown on screen and copied is the source from before appending.
5. Keep the background transparent (`background: transparent`) — the UI paints the block's surface.

### The diagram block's look (no code-block header)
- **Remove** today's line with the `mermaid` name·`Source`·`Copy`. The block is one card: fill `color.surface`, border 1px `color.line`, corners `radius.card` (12), padding top `space.7` (48 — room for the tool row) · sides·bottom `space.4` (16).
- The diagram is centred and shrinks to fit the block's width, but **never grows** (never stretched beyond its own size).
- **Source·Copy**: two small text buttons at `space.2` (8) from the top right (`type.meta`, fill `color.sunken`, text `color.muted`, corners `radius.sm`, height 24 or more). They appear in `motion.duration.fast` (120ms) on hover or when keyboard focus comes in, and are hidden otherwise. **On touch-only or keyboard-only screens they always show** (hidden, they couldn't be opened). Pressing Source turns the same block into code (`type.code`, fill `color.sunken`), and pressing again goes back to the diagram. Copy copies the source from before appending.
- Alternative text under the picture (accessibility): the diagram's title on one line as `aria-label`. Without a title, "Diagram".
- If drawing fails, it shows as the source code block, as now (with no header, one error line in `color.err`).
- Pictures for checking the look: `preview/mermaid-{flow,sequence,pie}-{360,720}.png` (light on top, dark below).

### The renderer's limits (`mermaid-rs-renderer` 0.3, checked)
- Two text styles (weight·colour) within one node aren't possible → hierarchy comes from fill·border colour and symbols.
- In sequence diagrams the arrow colour comes out paler than the theme value in dark (fixed in the renderer). Readable, but 3:1 not confirmed (unconfirmed).
- The pictures above are SVGs drawn by Chrome. Whether GPUI's SVG renderer uses the same font widths has to be seen on the desktop (unconfirmed) → checked in T-338's design QA.

## 2. Three HTML card templates

`node cards/card.mjs <board|compare|figure> <data.json> [--mode light|dark] > card.html` — fill in only the data and out comes **one HTML file with no script·external font·network**. All text is escaped, and the page carries `Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; img-src data:`. Without `--mode` it follows the screen's light·dark. The width follows the parent's (under 560px = phone layout, above = conversation width). The font is the tokens' stack (`Inter` → a family for each script, e.g. `Pretendard` for Korean → the system) and only installed fonts are used — a script whose family isn't on the rendering daemon's Mac comes out in the system's font for that script (unconfirmed). The card's fixed words (`As of`, `None`, `Recommended`, the status names) are English in this reference; a member writes its card in the person's language.

- **board — status board** `{title, subtitle?, asOf, source?, projects:[{name, work:[item], next:[item], wait:[item]}]}` · item `{id?, title, who?, note?}`. Three rows per project: in progress · next · your turn. Empty is "None". A your-turn `note` says in one sentence what the person has to do.
- **compare — comparison table** `{title, subtitle?, options:[{name, pick?}], rows:[{label, values:[…]}], why?, asOf?}`. The `pick` column gets a tinted fill + ★ Recommended, `why` is the reason for the recommendation in one sentence. On a narrow width the table becomes groups per item (option name before each value).
- **figure — number summary** `{title, asOf?, source?, stats:[{label, value, unit?, delta?, deltaTone?:'good'|'bad'|'flat', points?:[numbers], from?, to?}]}`, 1–3 of them. Changes are the ▲ ▼ – symbols + words (never colour alone). With 2 or more `points`, a sparkline (`color.accent` 2px; never invent values — real values only).
- Every card carries `asOf` ("As of Nov 3") at the bottom. Numbers are `tabular-nums`.
- The sample data is this squad's 2026-11-03 board (in progress·next·your turn) and the `git log` of merges to main (10/28–11/3, 17), in English. The `compare` example is the T-335 contract candidates' comparison.
- PNG check: `equipe design render <html…> --widths 360,720` → `preview/{board,compare,figure}-{light,dark}-{360,720}.png`.

## 3. Contrast (computed, `contrast.md`)
Every pair the cards and Mermaid use (about 50 pairs each for light·dark) was computed, with 0 below AA — text 4.5:1, lines·borders·slices 3:1.

## 4. What this folder doesn't do
- No new tokens (the pie colours too are the existing status·accent tokens, only ordered).
- The daemon baking HTML into PNG is T-337; showing it in the conversation is T-338·T-339.
