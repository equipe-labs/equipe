# equipe brand strategy (one page) — v1 proposal

Status: **proposal** — a draft before the person confirms it (playbook B2 ★). Facts carry their sources; judgements without grounds are marked "proposed".
Sources of fact: `equipe-site/design/copy.md` (the site's canonical copy), `equipe/docs/BRAND.md`, `equipe-app/docs/mvp-flows.md` §1, squad memory `site-primary-cta-macos-app`·`design-system-and-brand`.

## 1. Positioning

**Who it's for**: **one person who wants to lead agents like a small team** instead of directing them one at a time, and already uses Codex or Claude Code (developers·founders). Source: copy.md "Who equipe is for".
**What it does for them**: agents by role (PM·designer·frontend·backend·mobile…) work in parallel on a board, each on its own branch, and **a merge happens only after the person approves**.
**Against the alternatives**: not another agent loop but a **team layer** over the agent engines already there (copy.md "Support line").

> **One sentence (proposed)**: equipe is an AI software squad led by one person — the agents work in parallel, the decisions stay yours.

**Role of each surface (with the person's decision of 2026-10-16)**: the primary action = downloading the macOS app; installing the CLI is secondary. The phone app is *a remote for approving·reviewing* (App Store line: "Approve your computer's AI squad from your phone").

**One more thing — the site has two H1s.** `copy.md` says "Your AI software squad, led by you.", the live site "Lead a team of AI agents. You approve every merge." (audit V1). The core both share with this strategy's sentence is *"lead"* and *"you approve"*. Which one is canonical is the person's decision → recorded in `guidelines.md` §5.3.

## 2. Personality (proposed — 4) and 3 "not"s

| Trait | Meaning | Grounds |
|---|---|---|
| **Calm** | No shouting·hype·exclamation marks. The agents do the work; the person decides quietly | copy.md "plain, concrete, a little dry. No hype words"; BRAND.md "silence" |
| **Precise** | Concrete nouns·numbers·commands. Not "fast" but what, and how | copy.md "Nothing below goes beyond [the README]" |
| **Editorial** | Space and hierarchy, no ornament. Restraint that shows the maker's hand | BRAND.md "Editorial, restrained… lots of negative space" |
| **On your side** | Authority always stays with the person. When stuck, it says exactly what the person must do | the "Your approval" section, the app's `mvp-flows.md` §1 "Never act on stale or unclear information" |

**Not**: ① flashy·"innovative" (hype) ② cute·mascots·emoji (BRAND.md "never cute… no emoji") ③ a cold machine — the impression that 'the AI just does it all' (a tone that takes authority away).
Note: BRAND.md goes as far as "cold-luxury, a little uncanny", but what product screens (site·app) actually use is calm and editorial. The proposal is to keep the 'fashion house' mood to *brand surfaces* like social cards·avatars and not bring it into the UI.

## 3. Voice·tone

**Shared rules**: short, active voice, concrete. What the person does comes first. In a sentence the name is always lowercase `equipe` (no dot). Commands·file paths in code format.

| Situation | Tone | Example (EN) |
|---|---|---|
| Introduction·hero | stated plainly | "Lead a team of AI agents. You approve every merge." |
| Asking for approval | direct, only what's needed | "Allow editing files?" · "Deny" |
| Something the person is holding up | what to do + why | "frontend is waiting — once you send, it picks up where it left off." |
| Error | facts + the next action instead of apologies | "Your Mac is offline. Check that equipe is running." |
| Empty state | guidance, one step | "No squads yet. Create one on your Mac with `equipe`." |
| Notification | only who needs what (no commands·content — T-62) | "backend needs you" |
| Release news | facts only, with a date | "v0.30 — Desktop app updates itself." |

Every other language writes the same lines in its own register and the glossary's terms (below); its strings live in each app's locale files, not here.

Example sources: "frontend is waiting…" comes from a desktop screen (`desktop-tickets.png`), and the release-news version number is **an example**. The rest of the English lines are **proposed sentences** written after `mvp-flows`·the app's English strings·copy.md's phrasing (not quoted verbatim). The Android English error line "Your Mac is offline. Check that equipe is running." is taken as is from `values/strings.xml`.

**Don't**
- Hype words: *revolutionary, 10x, magic, supercharge* — and their equivalents in every language
- Exclamation marks, emoji, ALL CAPS decoration (copy like `APPROVE`)
- Personification·jokes ("Oops!" and the like, with or without 😅)
- Wording that hides the person's decision, like "the AI takes care of it" (authority stays with the person)
- Vague promises ("see you soon") and promoting features that don't exist (also against store rule 2.3.1) — what's coming is written "coming"
- Different names for the same idea: squad, member, ticket, board, approve — the canonical terms are in `equipe-site/design/i18n/glossary.md` (8 languages)

**Register in each language (proposed)**: one register per language, everywhere. For Korean, **the polite -yo register (haeyo-che) throughout** — the site·store already use it, the app mixes haeyo-che and the formal hapsyo-che (audit V3). Grounds: "plain, a little dry" holds without the formal register's stiffness, and it suits a phone screen's short sentences. The second-person "you" (dangsin) only in promotional·introductory copy; the app UI drops the subject by default. Other languages set theirs in the glossary the same way. Awaiting the person's decision.

## 4. The visual language in a line (details in `guidelines.md`)

**One blue dot on warm neutrals (bone·void).** Colour is barely used; the colours used are meaning (status) and the one brand blue. Space and hierarchy instead of ornament.

## T-617 binding brief — document focus view

- **Problem:** A long Markdown report is trapped in the right panel, especially at the 760pt minimum window width. The person must be able to read it across the app content area and return to the exact conversation/board state.
- **Success:** Focus entry is discoverable from the document header; the focused header makes the document and return action unambiguous; `Esc`, the header action, and one toggle command all restore layout, document, scroll, and keyboard focus. Korean and English long titles remain available at 760/1200pt and 100/150% text.
- **Reference:** Linear Peek for opening detail without losing list context; Raycast navigation for one consistent `Esc` meaning. Take the reversible context transition and keyboard visibility, not either product's brand or copy.
- **Locked:** existing warm-neutral palette, system typography, current Markdown/Mermaid renderer, 760/1200pt support, light/dark, quiet tool density, no decorative AI effects, no macOS fullscreen change.
- **Free:** focus header structure, entry/exit placement and labels, body measure and responsive header wrapping.
- **Visual thesis:** A reading surface that feels like the same document gently taking the room, with the way back always clearer than the mode itself.
