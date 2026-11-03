# Visuals in the conversation

A member answers with a picture where a picture is quicker to grasp than words — where things
stand, a comparison, a flow, numbers — and every screen shows it as a picture: the desktop, the
iOS and Android apps, the terminal. This is the contract between what members write, what the
daemon does with it and what each screen shows. The person decided it on 2026-11-03: diagrams,
charts and tables, and HTML cards later; only where they help; every member, every screen.

The rule that holds everywhere: **a visual never hides the words, and one that can't be drawn
shows its source.** A screen never shows an empty box, a spinner forever or an error in place of
a diagram — it shows the block as text, as it was written.

## What members write

All of it is plain Markdown in a reply. Nothing is a new tool call; a reply with visuals is still
a reply, folded and journaled as one (`docs/remote-protocol.md`, *Folding*).

| Form | Written as | For |
|---|---|---|
| Diagram or chart | a fenced block whose info string's first word is `mermaid` | flows, steps, timelines, where things stand, numbers |
| Table | a GitHub-flavoured Markdown table | comparisons, status |
| Picture | `![label](/absolute/path.png)` | a render, a screenshot, a chart drawn by a script |
| HTML card | a fenced block whose info string is `html card` | a status board, a comparison, numbers at a glance: see [HTML cards](#html-cards) |

### Mermaid

```` ```mermaid ```` (or `~~~mermaid`, any fence of three or more, `Mermaid` too) up to its
closing fence — a fence of the same character, at least as long, nothing after it. Indented at
most three spaces, as CommonMark has it. A block not closed yet (still streaming) is not a
diagram yet.

What draws is what [mermaid-rs-renderer](https://crates.io/crates/mermaid-rs-renderer) 0.3
draws — among them `flowchart`/`graph`, `sequenceDiagram`, `timeline`, `xychart-beta`, `pie`,
`quadrantChart`, `mindmap`, `gantt`, `stateDiagram`, `classDiagram`, `erDiagram`, `journey`.
Members are told to use flowchart, sequenceDiagram, timeline, xychart-beta and pie. Anything that
doesn't parse shows as its source.

A block's **key** is how a screen finds the picture drawn for it: the SHA-256 of the block's
source — the lines between the fences, joined with `\n`, trimmed of whitespace at both ends —
its first 8 bytes in lowercase hex (16 characters). `pie` → `558211ed72b2d696`.

### Tables

GitHub-flavoured Markdown: a header row, a `|---|` row, then rows. Every screen draws them
itself; nothing goes through the daemon. A screen too narrow for one scrolls it sideways, it
never wraps cells into a mess.

### Pictures

Unchanged: see *A member's pictures* → *Pictures in the conversation* in
`docs/remote-protocol.md`. A chart a member draws with a script is a picture of this kind.

### HTML cards

The daemon draws a card as PNGs, light and dark, like a diagram ([How a card is
drawn](#how-a-card-is-drawn)) — **but only once it is turned on**: it is off until the security
review (T-340) says otherwise, and members are not yet told to write one. While it is off, or
where a card doesn't draw, a screen shows the block as code, like any other.

- A fenced block whose info string is exactly `html card` (the words, in that order, any case).
  A block of plain `html` is code being shown, never a card.
- One HTML fragment, at most 32 KB: no `<html>`, `<head>` or `<body>`. It is the card's
  content: the daemon puts it in the design system's card (`<main class="vc"><article
  class="vc-card">…</article></main>`, with `design/build/css/tokens.css` and
  `design/visuals/cards/card.css`, T-336), so its classes (`vc-head`, `vc-title`, `badge`,
  `bd-*`, `cp-*`, `fg-*`, the `--eq-*` tokens) are there to use. The three templates — `board`,
  `compare`, `figure` — are what `node design/visuals/cards/card.mjs` puts in the `<article>`
  for its data (examples: `crates/core/src/visuals/card/examples/`).
- Nothing runs and nothing loads: no `<script>`, no `on…=` handlers, no `<iframe>`, `<object>`,
  `<embed>`, `<form>`, no `<link>`, no `http(s):` or `//` in `src`, `href` of a stylesheet or
  `url(…)`. A picture is a `data:` URL, or none. A screen that finds any of these shows the
  source instead.
- Styles inline or in one `<style>` in the fragment. Light and dark: the screen sets
  `--fg`, `--bg`, `--muted`, `--accent` and `--line` on the card's root and the card uses
  them (`color: var(--fg)`); a card may also use `@media (prefers-color-scheme: dark)`.
- As wide as the column (at most 640 CSS pixels), as tall as its content; a screen may cut it
  off past 1200 pixels with a "more" that shows the source.
- Links are shown as text; a screen never follows one from inside a card. An `<a>` or
  `<area>` (HTML's or SVG's) has no `href`, or one that is a place in the card (`#…`): anything
  else — `http(s):`, `//`, `file:`, `javascript:`, `data:`, a relative path, an empty `href`,
  one spelled with entities (`&#104;ttp…`) or whatever a browser strips around a URL — and any
  `target` or `download` fails the check (T-357). The check reads every `<` and a letter as a
  tag, inside a comment, a `<style>` or an attribute's value too, so a tag a browser finds where
  an eye wouldn't (after `--!>`, past a `</style>` a quote seemed to hold) is read as well. The
  card's page opened in the browser panel goes nowhere by click or key.

## Who draws what

| Screen | Mermaid | Tables | Pictures | HTML card |
|---|---|---|---|---|
| Desktop | draws it itself (same renderer, same themes — core's `visuals::diagram::svg`) | itself | `read_image` | the daemon's PNG; a click opens its page in the browser panel |
| iOS, Android | the daemon's PNG (`visuals_rendered`), light or dark as the phone is | itself | `read_image` | the daemon's PNG |
| Terminal (TUI, `equipe try`) | source, as a code block | as text | path | source |
| Telegram | source | as text | — | source |

A screen that draws Mermaid itself ignores `visuals_rendered`.

## The daemon draws for the screens that can't

When a reply line closes (`docs/remote-protocol.md`, *Folding*: the next line, `turn_ended`,
`work_ended`, `session_closed`), the daemon looks for Mermaid blocks in it and, off its loop,
draws each one twice — light and dark, in the same themes as the desktop — to PNG on a
transparent ground, at twice its size, the Mac's fonts for its words (Korean, Japanese and
Chinese labels draw).

Drawn files are kept in the squad's folder of the daemon's: `~/.equipe/squads/<squad>/visuals/
<key>-light.png` and `<key>-dark.png`. A key drawn before is read back, not drawn again.

Then every screen following the squad gets one event per reply that had a block drawn:

```json
{ "type": "visuals_rendered", "session": 3, "visuals": [
  { "kind": "mermaid", "key": "558211ed72b2d696",
    "light": { "path": "/Users/me/.equipe/squads/acme/visuals/558211ed72b2d696-light.png", "width": 880, "height": 724 },
    "dark":  { "path": "/Users/me/.equipe/squads/acme/visuals/558211ed72b2d696-dark.png",  "width": 880, "height": 724 } }
] }
```

- `kind` is `mermaid` or `card` (more kinds may come: a screen ignores a kind it doesn't know,
  and shows that block's source). A card's key is made the same way, of the source between its
  fences.
- `page`, a card's only: the path of its page, kept beside its PNGs (`<key>.html`) — see
  [Opening a card](#opening-a-card).
- `width`/`height` are pixels, drawn at 2×: show it at half — `width / 2` logical points —
  or smaller to fit the column, never larger.
- It comes after the reply's line closed, usually right after `turn_ended`; a block that didn't
  draw is simply not in it, and a reply none of whose blocks drew sends no event.
- It is not a line: it isn't folded, isn't journaled, doesn't close an open reply.
- The same block in a later reply has the same key: a screen may reuse what it already read.

**A screen attaching later** finds the same in its snapshot: each session carries `visuals`,
the blocks of `recent`'s replies drawn so far (absent when none). Only drawn files are listed —
the daemon never draws while it builds a snapshot.

**The bytes** come with `read_image` `{ "squad", "path", "max"? }`, the path as the event gave
it. The daemon gives a file in the squad's `visuals/` by its name alone (`<16 hex>-light.png` or
`-dark.png`, nothing else, no other squad's). For a phone it makes the copy at most 720 pixels on
a side, smaller again until it fits one relay frame (the relay takes frames of at most 1 MiB), so
a diagram always comes in one answer. A diagram keeps its transparent ground (PNG, not JPEG).

### How a card is drawn

As a reply line closes, alongside its diagrams, each ```` ```html card ```` block that passes
the contract's check (the list under [HTML cards](#html-cards): no script, handler, frame,
form, `<link>`, `<meta>`, `<base>`, nothing loaded but a `data:` picture; a block that fails it
isn't drawn) is put in its page and photographed by a **headless Chrome started for that one
picture** — Google Chrome, Chrome for Testing or Chromium where they are installed, or
`EQUIPE_CHROME`. No Chrome: nothing drawn, sources show.

- **Its own profile, thrown away.** Each drawing makes a folder of its own in the temp folder
  (mode 700) for the page, the profile and the picture, and removes it after, however it ended.
  It never touches the person's Chrome profile or the members' (`~/.equipe/browser/`), nor the
  keychain (`--use-mock-keychain`).
- **Nothing runs.** The page's first element after its charset is a CSP,
  `default-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src data:; base-uri 'none'; form-action 'none'`:
  no script, no handler, no frame, nothing fetched.
- **Nothing goes out.** Chrome is started with every request — the page's and its own, loopback
  included (`--proxy-bypass-list=<-loopback>`) — sent to a proxy on a port nothing listens on,
  no name resolving (`--host-resolver-rules=MAP * ~NOTFOUND`), and its background traffic off
  (`--disable-background-networking`, `--disable-component-update`, no sync, no extensions).
  Either holds without the other: the tests show a card asking for pictures, stylesheets,
  `@import`, fonts, `fetch`, frames and a refresh from a server of their own reaches neither it
  nor the proxy, and a page without the CSP doesn't either.
- **Bounded.** 10 seconds per picture, then Chrome and all it started are killed (its process
  group) and nothing is drawn. A window of 640×1200 CSS pixels at 2×: the PNG is cut to the
  card's height, and what is below 1200 pixels is cut off.
- Light and dark: the page is set to `data-theme="light"`/`"dark"` (the tokens' own switch) and,
  for dark, Chrome's `--force-dark-mode`, so a card's `prefers-color-scheme: dark` holds too. The
  ground around the card's rounded corners is transparent.

**Turning it on** (off until T-340): `{"html_cards": true}` in `~/.equipe/visuals.json`, read
at every reply, or `EQUIPE_HTML_CARDS=1` in the daemon's environment.

### Opening a card

A card's page is kept as `~/.equipe/squads/<squad>/visuals/<key>.html`: the fragment in the
same frame and CSP, without a fixed tone (it follows the screen's). A screen gets it with
`read_image` by the `page` path, like a picture — the daemon gives a file of the squad's
`visuals/` named `<16 hex>.html` and nothing else — as `"mime": "text/html"`, its bytes in
`data` (base64), at most 64 KB.

The desktop shows the card's PNG; clicking it opens the page in equipe's browser panel as a
`data:text/html;base64,<data>` URL (the panel opens `data:` URLs but not `file://` ones, and
the desktop may be on another Mac). The CSP inside still holds there: nothing runs or loads.
And nothing in it leads elsewhere: its links only go to a place in it (the check), and a page
kept under an older, looser contract isn't given out (`read_image` refuses it) or listed in a
snapshot. A test opens a kept page in a real Chrome, as a `data:` URL and as its file, clicks
everything in it — plain, ⌘, ⇧, middle — and Tabs through it pressing Enter and Space: its top
frame never asks for another URL, no tab opens elsewhere, nothing reaches the network, while the
same page with a link out, past the check, is seen to go (`card/tests/panel.rs`).

### Limits

| | |
|---|---|
| Source of one block | Mermaid: at most 20 000 bytes; a card: 32 KB; longer shows as source |
| Blocks drawn per reply | the first 8 (by key, repeats once) |
| Drawn size | 2×, at most 2048 pixels on the longer side; a card 1280 wide, at most 2400 high |
| Drawing a card | 10 s, then Chrome is killed |
| One PNG | at most 4 MiB, else not kept and not told |
| To a phone | ≤ 720 px a side, and one relay frame |

## Showing it, and when it fails

A screen showing a reply with a Mermaid block:

1. While the reply streams, or until it has a picture for the block: the block's source as code
   (a screen may fold it to a few lines with a quiet "diagram" label).
2. Once `visuals_rendered` (or the snapshot's `visuals`) has the block's key: `read_image` the
   tone the screen is in, and show it in place of the block. Switching light/dark reads the other.
   Tapping it may open it full screen; the source stays one tap away ("source").
3. `read_image` fails (`no such file` — the cache was cleaned —, too big, anything): the source,
   as in 1. No retry loop; one retry when the person asks.

A daemon older than this sends no `visuals_rendered` and no `visuals`: phones show the source,
as before. A screen older than this ignores the event (unknown `type`) and the field.

## What members are told

Every role's charter (`crates/core/src/roles.rs`, `VISUALS`) has the same paragraph, with no
path or name of any squad: add a visual to an answer only where it is quicker to grasp than words
— where things stand, a comparison, a flow or an order of steps, numbers — as a Markdown table
or a Mermaid block; a short answer stays short, without one; a visual never stands in for the
words — the point in a line or two, then the picture; small, with short labels in the person's
language; every number from a source named, never invented; it should still read as text, since
where it can't be drawn its source shows. In-depth research goes to a report with its own
visuals, as before.

## Code

- `crates/core/src/visuals/diagram.rs` — finding blocks, keys, the themes, Mermaid → SVG → PNG,
  the cache, what a snapshot lists.
- `crates/core/src/visuals/card.rs` — HTML cards: finding them, the check, the page, Chrome,
  the cache, `enabled`. Its tests draw the three templates and try to reach out.
- `crates/core/src/runtime.rs` — `draw_visuals`: drawn off the loop as a reply line closes.
- `crates/core/src/visuals.rs` — `serve_with`: a drawn diagram or card given by its path, a
  card's page as `text/html`.
- `crates/protocol/src/lib.rs` — `Event::VisualsRendered`, `Rendered`, `RenderedImage`,
  `SessionInfo::visuals`.
- `docs/remote-vectors/messages.json` — `event: visuals rendered`, as a phone gets it.
