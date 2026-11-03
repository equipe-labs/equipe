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
| HTML card | a fenced block whose info string is `html card` | later: see [HTML cards](#html-cards) |

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

The format is fixed now so members and screens agree; **no screen draws it yet** and members are
not yet told to write one — that comes with the tickets that draw it. Until a screen draws it,
it shows the block as code, like any other.

- A fenced block whose info string is exactly `html card` (the words, in that order, any case).
  A block of plain `html` is code being shown, never a card.
- One HTML fragment, at most 32 KB: no `<html>`, `<head>` or `<body>`.
- Nothing runs and nothing loads: no `<script>`, no `on…=` handlers, no `<iframe>`, `<object>`,
  `<embed>`, `<form>`, no `<link>`, no `http(s):` or `//` in `src`, `href` of a stylesheet or
  `url(…)`. A picture is a `data:` URL, or none. A screen that finds any of these shows the
  source instead.
- Styles inline or in one `<style>` in the fragment. Light and dark: the screen sets
  `--fg`, `--bg`, `--muted`, `--accent` and `--line` on the card's root and the card uses
  them (`color: var(--fg)`); a card may also use `@media (prefers-color-scheme: dark)`.
- As wide as the column (at most 640 CSS pixels), as tall as its content; a screen may cut it
  off past 1200 pixels with a "more" that shows the source.
- Links are shown as text; a screen never follows one from inside a card.

## Who draws what

| Screen | Mermaid | Tables | Pictures | HTML card |
|---|---|---|---|---|
| Desktop | draws it itself (same renderer, same themes — core's `visuals::diagram::svg`) | itself | `read_image` | source, until it draws them |
| iOS, Android | the daemon's PNG (`visuals_rendered`), light or dark as the phone is | itself | `read_image` | source, until it draws them |
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

- `kind` is `mermaid` (more kinds may come: a screen ignores a kind it doesn't know, and shows
  that block's source).
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

### Limits

| | |
|---|---|
| Source of one block | at most 20 000 bytes; longer shows as source |
| Blocks drawn per reply | the first 8 (by key, repeats once) |
| Drawn size | 2×, at most 2048 pixels on the longer side |
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
- `crates/core/src/runtime.rs` — `draw_visuals`: drawn off the loop as a reply line closes.
- `crates/core/src/visuals.rs` — `serve_with`: a drawn diagram given by its path.
- `crates/protocol/src/lib.rs` — `Event::VisualsRendered`, `Rendered`, `RenderedImage`,
  `SessionInfo::visuals`.
- `docs/remote-vectors/messages.json` — `event: visuals rendered`, as a phone gets it.
