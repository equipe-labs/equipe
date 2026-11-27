---
name: brand-kit
description: Use for a product's brand step, a logo round with logo_round (six candidates, the gate, one critique, three options in one card), the person's pick, the kit, favicons, social image, palette and type as tokens.
---

# The brand step: from a brief to a kit

It is part of the first sprint of a product, before the screens are polished, and it ends in files in the project's
`design/`, not in a chat: the logo, a favicon set, a social image, a palette and a type pair, saved as tokens so every
screen picks them up. In the starter brand (the default: see `design-system`) tools do every step below that has a right
answer — `starter_brand` runs them in a scaffolded project — and your part is one short look: the brief's words and voice
in the product's language, and anything plainly wrong. The deep brand system does each step in full.

## 1. The brief, in a few lines

Name (the spelling the person gave), who it is for, three to five attributes each with what it is not, one or two
references they admire, one thing it must not look like, and any colour or letter they already love. Ask it with
tappable answers where the app offers them (see "Asking the person"). Write it to `design/brand.md`.

## 2. A logo round: one tool call

A new or changed logo is ONE `logo_round` call (equipe platform tool), not a loop of pictures, redraws and critiques.
Pass `about` with what the business is in the person's own words from the conversation, and `avoid` / `notes` when
they said what it must not look like or what they love. It writes the brief from the product's files (no letters when
the brand is read in several languages), makes six genuinely different candidates in one picture call, holds each to
the quality gate (legible at 16, 32 and 180 px, contrast on light and dark, at most three colours, no gradients, no
frame, clean edges, no letters in the symbol, not a duplicate), runs ONE critique of the whole round (unwanted
readings such as a keyhole or an avatar, accidental letters, look-alikes of known marks, fit, small-size clarity), and
draws the best three as they will be used: the site header in light and dark, a browser tab at real size, the app
icon. When the product has no mark of its own yet, the best is already on it. A round costs well under a dollar.

Never generate logo pictures with `generate_image` yourself, never draw or redraw a mark by hand, and never run
`equipe design critique` on logo candidates: the round has its own critique. An existing logo that only needs to
become clean vector (an imported site's PNG) is `logo_round` with `{"redraw": "<file>"}`: the same mark, no new concepts.
Outside equipe cloud, `npm run logo` in a scaffolded product does the same steps without the critique.

## 3. The person picks with one tap

Post the answer's `decision` with ONE `ask_decision` call: its options as given, the question and labels written in
the person's language, the pictures kept. Then hand in and end your turn; start nothing else on the logo before they
answer. When they answer:

- a letter: `logo_round {"pick": "<its id>"}` writes the kit (step 4) and you hand in;
- "rounder", "bolder", "simpler": `logo_round {"refine": {"option": "<id>", "how": "rounder"}}` makes that version by
  code, free; show it with its own card (new version or keep);
- none of these: `logo_round {"reject": "<their words>"}` runs one more round with their words in the brief. After two
  turned-down rounds it tells you to hand the ticket back as `cannot`; the next round then runs on a stronger model.

One piece of work makes at most three picture rounds. Note what they chose and why in `design/taste.md`.

## 4. The final mark and the kit

`logo_round {"pick": ...}` saves clean SVG (`viewBox`, no metadata, no embedded raster, fills from the palette) and
writes the kit. In `design/assets/logo/`:
`mark.svg`, `mark-on-dark.svg`, `wordmark.svg` (the name outlined in the brand's type), `lockup.svg`, `favicon.svg`, and
beside the site's `public/`: `favicon.ico`, `apple-touch-icon.png` (180), `icon-192.png`, `icon-512.png`,
`og-image.png` (1200×630) and `site.webmanifest`. In a scaffolded project `npm run brand` writes all of these from
`design/assets/logo/mark.svg`, `design/brand.md` and the tokens; elsewhere draw them with `equipe design` and the
browser. Wire them into the page's `<head>` (icons, manifest, `og:image`, `theme-color`), and look at the favicon at
16 px and the social image at the size a link preview shows.

## 5. Palette and type, saved as tokens

Derive the brand colour from the mark; add warm or cool neutrals that sit with it, the semantic colours, and dark
values, each text pair at 4.5:1 or better. Choose a heading and a body typeface under an open licence (Google Fonts,
Fontsource), set for the product's languages. Write them into `design/tokens.json` (the brand colour as
`color.brand.*` and the roles that alias it), regenerate the theme (`web-stack`), and judge them on a real screen.

## Two things the kit never does

- No small labels in capitals with wide letter-spacing (eyebrows, kickers, badges like "BEST MATCH"). Write "Best
  match" and set it apart by weight, size and colour, or a soft sentence-case pill. Capitals only when the chosen
  direction says so: `design/direction.json` has `"labels": "uppercase"` (or labels with `"case": "uppercase"`), as a
  luxury, brutalist or sports brand may.
- Full borders are fine; never a border on only one side: no left, top, right or bottom accent line, no underline
  under a pill or tab, no single-side inset shadow (inset 3px 0 0, inset 0 -2px 0). Mark a selected tab, chip or card
  with a fill, a full border or a soft shadow instead.

## Asking the person

The cloud app turns a final line `choices: A | B | C` into tappable buttons (each under 30 characters, one such
line per question, never repeated in the prose). Elsewhere, ask the same question in words. Ask only what the person
alone can answer (the name, who it is for, what they love or must avoid); a logo, a palette or a typeface you choose
yourself, show with your reasons, and they can change it.

## What to say about rights

An AI-made logo may not be protectable as a trademark or copyright in every country and can echo an existing mark.
Tell the person in one line when you show the logo, suggest a trademark search before they print or sign anything, and keep
the generating prompts in `design/brand.md` so the logo can be traced to how it was made.
