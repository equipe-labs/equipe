---
name: web-stack
description: Use when you start a web product, page or component set with no stack yet, theme a UI from the product's design tokens, or choose an icon library — the default stack, why, and how to theme it.
---

# The default web stack, and how to theme it

## What to start from

A new web UI — a greenfield product, or a person with no stack preference — is built on:

- **TypeScript + React**, built with Vite to a static `dist/` (plus a small Worker for an API when the product needs one).
- **Tailwind CSS v4**, its `@theme` block mapped to CSS variables.
- **shadcn/ui** components, copied into the project (`npx shadcn@latest add <name>`), restyled only through tokens.
- **lucide-react** icons. Tabler (`@tabler/icons-react`) and Phosphor (`@phosphor-icons/react`) are allowed when the
  brand wants their shape; pick one family per product and never mix two.

Why this and not something custom: shadcn components are code the project owns, so a designer's tokens restyle every
one of them and nothing hides behind a library's runtime; Tailwind v4's `@theme` makes the token the single source of
every colour, radius and font; the stack is the one models and people know best, so work is faster, references are
plentiful and there are fewer invented patterns; the build is static files, which any host serves, Cloudflare Workers
and Pages included; and one icon family has one stroke weight and one grid, which hand-picked icons never have. Icons
are never emoji and never one-off drawings unless the brand's own set says so; an icon-only button has an
accessible name.

{{#if role:frontend}}
If your platform tools include `scaffold_app`, start there: it writes this stack, themed and building, with the
tokens script, a first set of components and the `design/` folder. Add components with `npx shadcn@latest add`, not by
writing look-alikes.
{{/if}}
{{#if role:designer}}
If the platform tools include `scaffold_app`, ask for the project to be started from it: it writes this stack with
the tokens file already wired to the theme, so your tokens reach every component on the first build.
{{/if}}

## An existing project keeps its stack

Read `package.json`, `components.json` and the styles before you change anything. A project that already runs on Vue,
Svelte, plain CSS, another component library or another icon set stays on it; so does an imported product. Use the
same ideas in its terms (shadcn-vue, shadcn-svelte, its own variables) and propose a migration only when the person
asks for one or the stack is the reason the work can't be done.

## Theming: tokens are the source, the theme is generated

The project's tokens file (`tokens.json` in `design/`; W3C Design Tokens, light as `$value`, dark in `$extensions.mode.dark`) is the source. The
stylesheet the app loads — `src/theme.css` in a scaffolded project — is generated from it with `npm run tokens`, never
edited by hand, and read back with `npm run tokens -- --pull` when a theme was tuned in CSS (a shadcn theme builder, a
quick experiment), so the two never drift. By hand it is the same mapping:

| Token role | CSS variable | Tailwind utility |
|---|---|---|
| page, text | `--background`, `--foreground` | `bg-background`, `text-foreground` |
| card, popover (and their text) | `--card`, `--card-foreground`, `--popover`, `--popover-foreground` | `bg-card`, `bg-popover` |
| brand action | `--primary`, `--primary-foreground` | `bg-primary`, `text-primary-foreground` |
| quiet surfaces | `--secondary`, `--muted`, `--accent` (+ `-foreground`) | `bg-muted`, `text-muted-foreground` |
| danger | `--destructive` | `bg-destructive` |
| lines, fields, focus | `--border`, `--input`, `--ring` | `border-border`, `ring-ring` |
| corner radius | `--radius` | `rounded-lg`, `rounded-md` |
| data colours | `--chart-1` … `--chart-5` | `bg-chart-1` |
| type | `--font-sans`, `--font-heading`, `--font-mono` | `font-sans`, `font-heading` |

Light values on `:root`, dark on `.dark`, and an `@theme inline { --color-background: var(--background); … }` block so
the utilities exist. A component uses utilities and variables only: no hex, no pixel radius, no font name in a
component. Each foreground reaches 4.5:1 on its surface, and `--ring` 3:1 on the page, in light and dark (`equipe
design system` says which don't).

To change the brand, change the tokens, run the script and look at a real screen before and after; to change one
component, change its variants in the component file with the same variables.

## Shipping it

`npm run build` writes `dist/`. On equipe cloud, `deploy_site` uploads that folder (`spa: true` for a single-page
app); a Worker API goes beside it with `main`. Load the page and press its main action before you say it works.

## Measuring it

Every new screen, form and key action gets analytics events in the same ticket (your `measure` skill has the names and
rules). Two ways: `window.equipe?.track?.("signup_completed", { plan })` in code, or `data-eq-track="signup_completed"`
on the button. Guard with the optional chaining: `window.equipe` only exists on the deployed page, not in local dev.
After `deploy_site`, do the action once and check it in `query_analytics`.
