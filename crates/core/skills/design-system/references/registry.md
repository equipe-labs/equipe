# A component registry, and each component's life in it

## The registry

- **Tokens first.** The system's values live in its tokens (DTCG `tokens.json`, primitives → roles → components) and
  reach the code as the CSS variables a shadcn theme reads — `--background`, `--foreground`, `--primary`,
  `--primary-foreground`, `--muted`, `--border`, `--ring`, `--radius`, … on `:root`, their dark values on `.dark`,
  mapped to Tailwind with `@theme inline`. Write that stylesheet from the tokens yourself, or with an exporter where the
  project has one (Style Dictionary, or another tool the person uses); either way it is generated from the tokens, never
  edited apart from them. Every text colour's pair (`primary-foreground` on `primary`) reaches 4.5:1 in light and dark,
  and `--ring` 3:1 on `--background` — `equipe design system` says which don't.
- **`components.json`** at the project's root (`npx shadcn init` writes one): its style, the stylesheet with the theme
  (`tailwind.css`), and the aliases (`ui`: where components go, `lib`, `utils`, `hooks`).
- **`registry.json`** at the root: `name` (what projects call it: `@<name>/button`), `homepage`, and an item for each
  component, block, hook and helper — `name`, `type` (`registry:ui` for a component, `registry:block`, `registry:lib`,
  `registry:hook`, `registry:file` with a `target`), `title`, `description`, `files`, `dependencies` (every package its
  files import, with no version unless one is needed), `registryDependencies` and `meta`. An item that needs another of
  this registry's names it by URL (`<homepage>/r/utils.json`) or namespace (`@<name>/utils`) — a bare `utils` installs
  shadcn's own, not yours. `meta.source` says where it came from (the shadcn component's page it was built on, or
  `original`) and `meta.license` its source's licence (shadcn/ui is MIT: keep its licence text in the repository).
  Nothing of another brand ships in it: no shadcn or Vercel avatars and logos from their examples, no logo or font of
  someone else's — your own placeholders, and a licence noted for any picture or font it ships.
- **`shadcn build`** writes each item, its files' contents inside, to `public/r/<item>.json`, served at
  `<homepage>/r/<item>.json`; a project adds it with `npx shadcn add <homepage>/r/button.json`, or `@<name>/button` with
  the registry in its `components.json`'s `registries`.
- `equipe design registry --build` builds it and checks every item: files there, every import declared, the items it
  needs named so they install from here, source and licence, no other brand's pictures, the build no older than its
  source — then installs a component into a scratch project, offline from the build, and says whether every import in
  what it wrote resolves (`--try <item>` for others; `--npx` installs it with the real `npx shadcn add`, which needs the
  network).

## The full set

A system is whole with every component a product reaches for: accordion, alert, alert-dialog, aspect-ratio, avatar,
badge, breadcrumb, button, calendar, card, carousel, chart, checkbox, collapsible, combobox, command, context-menu,
data-table, date-picker, dialog, drawer, dropdown-menu, form, hover-card, input, input-otp, label, menubar,
navigation-menu, pagination, popover, progress, radio-group, resizable, scroll-area, select, separator, sheet,
sidebar, skeleton, slider, sonner (toasts), switch, table, tabs, textarea, toggle, toggle-group, tooltip — and the
blocks the products need (a sign-in, a settings page, a dashboard shell), built only from them. `equipe design system`
gives the coverage; build the missing ones before a product needs them, not one-off in a screen.

## A component's life

1. **Spec** (designer): what it is for, its anatomy, every variant (`variant`, `size`, …) and every state its pattern
   has — at rest, hovered, reached by the keyboard, pressed, disabled, invalid, checked, open — light and dark, the
   tokens each part uses, its keyboard behaviour from the WAI-ARIA Authoring Practices pattern it follows, and its
   copy. A ticket for frontend when it is code.
2. **Build on the shadcn base**: start from shadcn/ui's source for it (`npx shadcn add <name>` into the registry's
   folder, or its page), keep its structure and its accessibility (Radix or the primitive it uses), and restyle it with
   the system's tokens only — no colour, radius, shadow or size written out. Variants in `cva`, each a value the spec
   names. Its item in `registry.json`, with `meta.source` and `meta.license`.
3. **Its docs page**, one per component, in the project's docs site: what it is for and when not to use it; an example
   of every variant, and of every state the matrix can't put it in itself; light and dark; usage code; and its
   accessibility notes (its role, its keys, what a screen reader says). Mark each example
   `data-example="variant=outline,size=sm"` (an `axis=value` for each variant it shows; one without an axis shows its
   default) and, where the part a person uses isn't the first control in it, that part `data-target`.
4. **Its matrix**: `equipe design matrix <name> --url http://localhost:<port>/docs/components/{name}` (or `system_tools`
   with the same words) draws every example in every state its pattern has, light and dark, at the widths you give
   (`--widths 390,1280`; all seven for a component that changes with the width), into one contact sheet a width in
   `design/checks/components/<name>/`; runs axe-core's WCAG A and AA rules on the examples; and a keyboard pass — Tab
   reaches it, focus shows, Enter and Space act, Esc closes what it opened and focus goes back, focus stays inside a
   modal, the arrow keys move in menus, tabs, radios and toolbars. It says in plain words what to fix: a variant the
   page doesn't show, a state it couldn't show or that looks no different, text under 4.5:1 in a state, axe's
   violations, a key that does nothing. Look at the sheet yourself; it isn't released until the matrix passes.
5. **Version and changelog**: the registry's version goes up for each release — a new component or variant a minor
   one, a change that breaks how a product uses it (a prop renamed, a variant removed) a major one, said with what
   to change — and `CHANGELOG.md` says, a line each, what changed in which component.
6. **Release**: `equipe design registry --build` with nothing left to fix, the coverage and the matrices passing, then
   the person accepts it like any other work; products update with `npx shadcn add` over their copy.

## Products only from it

A product screen uses the registry's components and the tokens, nothing beside them: no styled `<button>` where there
is a Button, no hand-made modal where there is a Dialog, no Radix or other UI kit imported around the registry, no
value written out that a token holds. Where the project has a registry, equipe's lint holds every UI edit to that (the
`system-*` rules): each comes back to you as you edit, and it blocks a hand-in as the floor does. When the system lacks something a screen needs, the
designer adds it to the system — a variant, a component, a token — and the screen uses it; a real exception (a
partner's required button, an old screen kept until its ticket) is kept with its reason and scope in
`design/decisions/` (`equipe design lint --waive system-component --scope <file> --why "…"`), never by default.
