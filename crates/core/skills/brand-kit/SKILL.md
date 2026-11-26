---
name: brand-kit
description: Use for a product's brand step — logo candidates that passed the quality gate, the designer's pick (the person can switch), the final mark, wordmark, favicon set, social image, palette and type saved as tokens.
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

## 2. Candidates that passed the gate

Make four genuinely different candidates, not four variations of one: a symbol made of simple geometry; an abstract
mark; a letterform or monogram; an emblem with one clear idea. In a scaffolded project the tool does this already:
`npm run logo` (started when the product was made) generates them, traces raster pictures to clean SVG, and rejects
every one that fails its quality gate — legible at 16, 32 and 180 px, contrast on light and dark, at most three
colours and a path limit, no gradients, no frame, no specks or jagged edges, no letters in the symbol, the wordmark
read back as the name. If fewer than two pass it tries once more, then falls back to typeset monograms, so there is
always a logo that passed. `design/concepts/options.json` and `logo-qa.md` say what passed and why. Never make concepts
by hand with the image tool or draw a logo yourself; `npm run logo -- --force` makes a fresh set for cents.

Elsewhere, use the image tool your platform gives you (on equipe cloud, `generate_image` with `purpose: "logo"`) and
hold every candidate to the same checks: show each at 32 px and 16 px on light and dark, run the critic first as
`design-process` says, and drop any that is a raster in an SVG wrapper (`<image>`), has gradients, needs more than about
thirty paths, misspells, or looks like a known brand (search the web for the mark and for the name). Prompt like a
brand designer, name no existing brand, artist or character, ask for the symbol alone and set the name yourself in a
licensed typeface. Note on the ticket what a round costs before you start (for the team; never a price in what the person reads).

## 3. The team picks, the person can switch

Pick the strongest of the candidates that passed, against the direction in `design/DESIGN.md`, yourself: in a
scaffolded project keep the tool's pick or switch with `npm run logo -- --pick <letter> --by designer --why "<one line
tied to the direction>"`. Then show the person the passing options with your pick marked and why, in one message ("I
went with B: it stays clear as a tab icon and carries the warm, hand-made feel; you can switch on the Brand page").
Never ask them to choose first. If they switch (the Brand page does it in one tap), that is the logo; note what they
chose and why in `design/taste.md`.

## 4. The final mark and the kit

Refine the pick with one or two passes (simplify, even out the weights, fix the spacing) and save clean SVG:
`viewBox`, no metadata, no embedded raster, fills from the palette or `currentColor`. Then, in `design/assets/logo/`:
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
