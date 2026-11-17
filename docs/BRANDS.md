# Brands: one design system, many projects

A **brand** owns a design system — tokens, components, brief, taste and
references. Its **projects** (a site, an app, a landing page, an internal
tool) inherit it and override only where they must differ. A project without
a brand keeps its own system, exactly as before ([DESIGN-SYSTEM.md](DESIGN-SYSTEM.md)).

Brands are local: they live on the Mac, not in one squad, so several squads
can share one. Nothing here touches the network.

## Layout

```
~/.equipe/brands/<slug>/              machine-level, shared by squads
  brand.json                          {"slug": "kettle", "name": "Kettle", "version": "2"}
  design/
    brand.md                          who it is for, this-not-that, voice
    tokens/*.json                     W3C Design Tokens (DTCG), laid in name order
    components/                       previews and rules, as in a project's design/
  taste.md                            the brand's taste profile
  refs.md                             references: links and notes, never others' screenshots
  versions/<version>/                 each released version, kept whole and never changed

<project>/design/brand.json           {"brand": "kettle", "pin": "2"}   (optional)
<project>/design/tokens/*.json        this project's overrides only (same DTCG paths)
```

A squad lists the brands it uses (kept in its journal); `equipe design brand
show <slug>` names the squads that list one.

## Resolution

A project's system is the brand's tokens with the project's overrides laid
over them, **path by path, the project winning**:

- groups merge; a token the project has replaces the brand's **whole**
  (its `$value`, `$type`, and dark value in `$extensions.mode.dark`);
- tokens only the brand has are inherited unchanged; tokens only the project
  has are added (and `equipe design check` warns, in case the brand dropped one);
- aliases (`{color.primary}`) resolve across both, so an alias the brand
  keeps follows the project's override.

The pin picks the brand version: `versions/<pin>/` when released, the brand
as it is when the pin is its current version. A link without a pin follows
the brand as it is. A project pinned to an old version keeps working, and its
members are told a newer one exists.

The resolved system is what `tokens.json` is read as everywhere equipe reads
it: the Design system screen, `equipe design check` (contrast included), the
lint and design QA's precheck, the system found in code (`equipe design
system`, its token contrast), the specimen, and `equipe design emit`. An
override that breaks WCAG AA fails those checks even when the brand passes.

**No `design/brand.json` → nothing changes:** `tokens.json` is read byte for
byte as it is, and `design/tokens/` means what it meant before.

A link that doesn't resolve (no such brand, no such version) is an error in
`equipe design check`; the project's own `tokens.json` is read meanwhile.

## Who changes what

Members working on a brand-linked project are told the system is
**brand-owned**: change the brand only on a brand ticket (one the PM flags
`Design: brand`, or that asks for a rebrand — `design::brand`). On any other
ticket only the project's overrides change. On a brand ticket the designer
changes the brand's folder, then releases it.

## Commands

| Command | What it does |
|---|---|
| `equipe design brand new <slug> [--name N] [--from design/] [--squad S]` | Makes the brand, starting from a design folder's tokens or the starter set; releases v1 |
| `equipe design brand release <slug> [--version V]` | Keeps the brand as it is now as a new version (default: the next whole number) |
| `equipe design brand link <slug> [design/] [--pin V \| --latest] [--squad S] [--squad-only]` | Writes the project's `design/brand.json` (pinned to the current version unless told) and lists the brand for its squad |
| `equipe design brand unlink [design/]` | Removes the project's link; its overrides stay |
| `equipe design brand unlink --squad S --brand <slug>` | Takes a brand off a squad's list |
| `equipe design brand show [--dir design/] [--json]` | The project's brand and version, and which tokens are overridden, its own, and inherited |
| `equipe design brand show <slug>` | The brand: versions and the squads using it |
| `equipe design emit [design/] [--out file]` | The resolved `tokens.json` (unchanged for a project without a brand) — e.g. `--out design/tokens.json` for other tools |

Code: `crates/core/src/design/inherit.rs` (resolution, versions, links),
`tokens::layer` (the path-wise merge), `crates/cli/src/design_brand.rs`.

## Later

The equipe Hub (stage 3) syncs a brand folder to a team's private space; the
local folder stays the source of truth offline. Moving the squad's
`design-taste` memory into a brand's `taste.md` is a separate, person-confirmed step.
