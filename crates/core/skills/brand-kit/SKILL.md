---
name: brand-kit
description: Use for a product's brand step — logo options from an image model, the person's pick, then the final mark, wordmark, favicon set, social image, palette and type saved as tokens.
---

# The brand step: from a brief to a kit

It is part of the first sprint of a product, right after the brief and before the screens are polished, and it ends in
files in the project's `design/`, not in a chat: the logo, a favicon set, a social image, a palette and a type pair,
saved as tokens so every screen picks them up.

## 1. The brief, in a few lines

Name (the spelling the person gave), who it is for, three to five attributes each with what it is not, one or two
references they admire, one thing it must not look like, and any colour or letter they already love. Ask it with
tappable answers where the app offers them (see "Asking the person"). Write it to `design/brand.md`.

## 2. Four concepts, genuinely different

Make four directions, not four variations of one: a symbol made of simple geometry; a letterform or monogram; a
pictorial mark with one clear idea (what the product does, or who it is for); a wordmark-led option with a small
accent. Use the image tool your platform gives you — on equipe cloud, `generate_image` with `purpose: "logo"`. It
routes to a vector-capable model (SVG output where one is configured) and falls back to a raster model, and its
answer tells you the provider, the files it saved and what it cost. Prefer vector: a raster logo can't be a favicon,
a wordmark and a one-colour print version without being redrawn.

Prompt like a brand designer: "flat vector logo mark for <what it is>, <attribute words>, simple geometric shapes,
two or three solid colours, no gradients, no text, no mockup, no shadows, centred on a plain background, reads at
16 px". Name no existing brand, artist or character. Image models misspell: ask for the symbol alone, and set the
name yourself in a licensed typeface — the kit script outlines it (below). Use a text-in-image model (Ideogram) only
when the person wants lettering drawn as part of the mark, and proofread every letter.

Say what one round will cost before you start (the tool reports each call's cost; four vector concepts are well under
a dollar), and stop after two rounds to ask rather than burn credit on taste.

Show them on one sheet, large, at 32 px and 16 px, on light and on dark. Run the critic first, as `design-process`
says. Drop any option that is a raster in an SVG wrapper (`<image>`), has gradients it can't lose, needs more than
about thirty paths, or looks like a known brand: search the web for the mark and for the name before the person
falls for it, and say plainly if something is close.

## 3. The person picks

Offer the options as letters on the sheet, with your recommendation and why, and nothing else to decide:
`choices: A | B | C | D | None, try again`. The pick is theirs; remember what they chose and why in `design/taste.md`.
"None" is one more round, with their words in the prompt, never a longer argument.

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

## Asking the person

The cloud app turns a final line `choices: A | B | C` into tappable buttons (each under 30 characters, one such
line per question, never repeated in the prose). Elsewhere, ask the same question in words. A logo, a palette or a
typeface is the person's call: give them options with your reasons; don't decide for them.

## What to say about rights

An AI-made logo may not be protectable as a trademark or copyright in every country and can echo an existing mark.
Tell the person in one line when they pick, suggest a trademark search before they print or sign anything, and keep
the generating prompts in `design/brand.md` so the logo can be traced to how it was made.
