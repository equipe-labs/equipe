# T-642 · Renaming a conversation — implementation contract

The person renames one PM conversation (including a topic title equipe generated, T-578). Editing happens in **one small card hanging under the header title**; there are three ways in, but only one place to edit. The conversation stays visible behind it (not a modal).

- Pictures: UI map `conversation/rename` (since T-643 the app itself, shot by `design/flows/shoot-desktop.py`), and the eight-language card sheet `langs-sheet.png`.
- Source of the original mockups: `prototype.html` (`?state=…&lang=…&mode=…&w=…&zoom=…`) over real app snapshots (`base/`, the T-578 build), with only the changing parts redrawn. Redraw with `node design/specs/rename-conversation/render.mjs` and `langs.mjs`.
- Mockup colours and type are the nearest design-system tokens. The implementation uses the `theme.rs` palette and existing components (mapping below).

## Options not chosen

- Editing inside the sidebar row (as ChatGPT and Claude.ai do): with the sidebar hidden, or the row off screen (760×600), there is nowhere to edit. At 150% a 244 pt row has no room for an error message.
- Editing in place in the header (as Finder does): the most direct at 1200, but at 760 or 150% the header has about 120 pt left, too little for a field, a counter, an error and buttons. It would need two layouts, wide and narrow.
- Chosen: a card attached to the header title. One layout at any width, and errors and retry happen in the same place.

## Entry points

| Path | Pointer | Keyboard |
|---|---|---|
| Header title | The title is a button. On hover: `p.wash` background and a pencil after the title (12 pt, `p.muted`, 8 apart). Tooltip `Rename`. If the title is cut, the tooltip's first line is the full title (with `Project / title` when the project is folded), the second `Rename` | Tab order: project → **title** → ⋯. Return or Space opens the card |
| This conversation ⋯ | Menu **first item** `Rename` (pencil icon), a rule under it, then the existing `Move to another project`, the projects, a rule and `Put away` | ↑↓ move, Return picks. Opening the menu focuses the first item |
| Sidebar conversation row | On hover, two buttons at the right: pencil and × (24 pt each, 4 apart). Pencil tooltip `Rename`. Clicking it opens that conversation and the header card | — (Return on the row opens the conversation, as now) |

- No global shortcut (and no key shown in the menu).
- A cut sidebar row title shows the full title in the row's hover tooltip (outside the icons; T-641 note 2).
- A conversation not yet begun (`draft_talk`) and the PM's own conversation (`with_pm`, no menu) have no entry point: only where ⋯ exists. A read-only connection is handled like `Move conversation` (a refused command follows "Failure" below).

## The card

- Position: 6 pt under the header title button; its left edge **always at the title button's left edge** (unless fewer than 240 pt remain to the window's right, then moved left by that much, never left of the sidebar edge + 8 pt). Width `min(360, window right − 8 − left)`. It never leaves the conversation pane. A `deferred` popover shaped like the existing `talk-menu-list`: `p.raised`, 1 px `p.hair`, `R_MD` (8), `shadow_lg`, 12 inside.
- While open, the header title button keeps the `p.wash` background (what is being changed).
- Top to bottom:
  1. Label `Conversation name` (12 pt, `p.muted`); also the field's accessible name.
  2. Field: one-line `Input`, height `FIELD` (36), `R_SM`, `p.code_bg`, 1 px `p.hair`. Focused, its border is `p.accent`; in error, `p.danger()`. On open **the whole current title is selected**, cursor at the end. From 30 to 40 characters, a count `n/40` sits inside the field at the right (12 pt, tabular, `p.muted`). Past 40 the count moves to the right of the message line in `p.danger()` (so it takes no room from the field). When the text is longer than the field and scrolls left, the left 8 pt stays empty and 8→48 pt fades, so a half-cut glyph does not look broken.
  3. Message line: only when there is something to say (the card grows one line downward; its top does not move). 12 pt `p.danger()` with `CircleAlert` 13 pt.
  4. Buttons, right-aligned: `Cancel Esc` (Secondary) · `Save ↩` (Primary, `data-primary`). When the value cannot be saved, Primary looks disabled.
- Narrower than 300 pt (760 at 150%): the key hints (`Esc`, `↩`) go so both buttons stay on one line, and the count sits on the message line. Keys and accessible names are unchanged. If they still do not fit (German and the like), the two buttons stack at card width with **the primary on top**.
- Key hint colours: inside Secondary `p.muted` (6.29:1 on the white card, 6.37:1 dark); inside Primary `p.primary_key()`.
- While the card is open in a narrow header (760 at 150%), the project mark and `/` hide first to give the title room.

## Value and checks

- At most **40 characters** (the same cap as T-578's generated titles).
- Before saving, line breaks and tabs become spaces, runs of spaces become one, and leading and trailing spaces go. The field's own text is not changed.
- Empty or only spaces: while deleting, Save is just disabled. On Return or an outside click, the message `Enter a name.` and the field's error border show (UI map `empty` is after Return).
- Past 40 characters (typed or pasted): **nothing is cut.** From the 41st character on, `p.danger_bg()` background with `p.danger()` text; message `Up to {max} characters. Remove {over}.`; Save disabled.
- If the cleaned value equals the current title, saving just closes the card, sending nothing.
- Pasting accepts several lines (they become one by the rule above).

## Keys and focus

- When the card opens, the field has focus. Tab order: field → Cancel → Save → field (it cycles inside the card).
- Return (or ⌘Return): save. If the value cannot be saved, nothing is sent and the message shows; the card stays open.
- Esc: cancel — the card closes, the title as it was. Focus returns to where the card was opened from: the **header title button** for the header and ⋯, and also for the sidebar pencil (that conversation is open by then).
- **IME composition** (Korean, Japanese, Chinese): while there is marked text, Return only commits the composition and Esc only cancels it. Save and cancel act only without a composition. Composing characters count in the shown count, but checks use the committed value.
- **Outside click = Return**: nothing changed, it closes. Changed and savable, it saves. Not savable, the card stays open with the reason (the input is never thrown away).
- Moving to another conversation or screen (a sidebar click, ⌘N, back…) acts as an outside click, except that an unsavable draft is dropped and the original title kept (the header now belongs to another conversation).
- While the card is open, the ⋯ menu and the member popover do not open (one popover at a time, the existing `board.popover` rule).

## Save, success, failure

- Order: `RenameConversation { squad, session, title }` with the cleaned value → **at once** the header, sidebar and put-away list show the new title (optimistic) → the card closes → focus on the header title button (ring visible, state `saved`).
- When the daemon's confirming event arrives, nothing more changes. Other screens (another window, the phone) change with the same event.
- Failure (command refused, connection lost, no such session): the header and sidebar **go back to the original title**, and the card opens again with what the person typed (no selection, cursor at the end) and a message. The primary button reads `Try again ↩`. The count hides whatever the length (the message gets the room).
  - Connection lost: `Lost the connection to your Mac, so the name wasn’t saved. What you typed is kept.`
  - Anything else: `Couldn’t save the name. What you typed is kept.`
- Esc in the failure state cancels as above (the original title stays).

## A narrow header (with T-651)

The title gets the room first. While the title is cut, things give way in this order: ① `context 31%` hides (never shown cut) → ② the project name folds to its mark (the project's mark, or an empty circle outside projects), its name on the pointer → ③ if it still overflows (760 at 150%), the mark and `/` hide too. The title keeps at least 96 pt; ⋯ and the terminal and browser buttons always stay. This order is also the fix rule for T-651 (the top bar overlapping at 150% and 760).

## Accessibility

- Header title: role button, name = the title, description = `Rename`, `aria-expanded` = card open.
- Card: role dialog, name `Rename conversation`. The field's name `Conversation name`. The error message describes the field and is announced when it appears (failure is role alert).
- On success, announce `Renamed: {title}`. Cancelling announces nothing.
- Sidebar pencil and ×: names `Rename` / `Put away`, at least 24 pt. Icon-only buttons show their names as tooltips.
- Contrast (computed, WCAG 2 relative luminance): error text `red`/`raised` light 6.84:1, dark 6.68:1 · label and count `muted`/`raised` 6.29:1, 6.37:1 · count inside the field `muted`/`code_bg` 5.56:1, 6.65:1. All AA (4.5:1) or better. The mockup token values: error 6.57:1 / 6.26:1.
- Reduce Motion: the card appears without motion (as existing popovers).

## Strings (new keys `talk.rename_*`; existing keys unchanged)

English below; every language's wording (en, ko, ja, zh-Hans, zh-Hant, de, fr, es) is in `crates/desktop/locales/<lang>.json` under the same keys.

| Key | en |
|---|---|
| `talk.rename` (menu, tooltip, description) | Rename |
| `talk.rename_dialog` | Rename conversation |
| `talk.rename_label` | Conversation name |
| Cancel / Save | existing `peek.cancel` / `board.save` |
| `talk.rename_retry` | Try again |
| `talk.rename_empty` | Enter a name. |
| `talk.rename_long` `{max}` `{over}` | Up to {max} characters. Remove {over}. |
| `talk.rename_offline` | Lost the connection to your Mac, so the name wasn’t saved. What you typed is kept. |
| `talk.rename_failed` | Couldn’t save the name. What you typed is kept. |
| `talk.renamed` `{title}` (announcement) | Renamed: {title} |
| `talk.rename_count` `{n}` `{max}` | {n}/{max} |

- Japanese messages should break between phrases, never mid-word. If GPUI line breaking does not know phrases, draw each sentence separately so breaks fall only after `。` — unverified in GPUI.
- es: the existing `sidebar.put_away` is `Guardar`, the same word as save, which confuses next to this card. Out of this ticket's scope and left as is; consider another word (`Quitar`, `Archivar`) when implementing (undecided).

## UI map states (`conversation/rename`)

| State | kind | What |
|---|---|---|
| entry-title | default | Header title on hover: pencil and tooltip |
| entry-menu | default | ⋯ menu, `Rename` first |
| entry-sidebar | default | Sidebar row on hover: pencil and ×, pencil tooltip |
| editing | default | Card open, whole title selected |
| composing | default | IME composing (one syllable committed, the next composing) — Return only commits |
| empty | error | After Return with an empty value: message, Save disabled |
| too-long | error | Past 40 characters: the count over, Save disabled |
| saved | success | Header and sidebar show the same new title; focus on the header title |
| failed | error | Connection lost: original title, typed value kept, `Try again` |
| refused | error | The daemon refused (T-643): the same as failed with the generic message |
| long | long | A long German title: the header gives way in order |
| large | default | 150%: at 760 the project mark gives way; the card stays inside the conversation pane |
| large-failed | error | 150% English failure (connection lost): a narrow card, buttons on one line without key hints |

760 is a 760×760 window (at 600 high the sidebar conversation row is off screen, so TP-1's "same title" cannot be seen).

## QA contract (T-644)

- TP-1: entry-title → editing → saved: header and sidebar change in the same frame. The same conversation in another window changes with the event.
- TP-2: without a mouse, Tab → title → Return → field → Return saves / Esc cancels, and focus returns to the header title. VoiceOver reads the `Rename conversation` dialog, the `Conversation name` field and the error text.
- TP-3: empty, only spaces, 41+ characters, multi-line paste, Return/Esc mid-composition: neither the title nor the input is lost.
- TP-5: stop the daemon and save → the original title comes back and the card keeps the typed value with the `failed` message. Start it again and `Try again` saves.
- TP-6: 8 languages × 760/1200 × light/dark × 100/150%: the header gives way in order, ⋯ never disappears, the card never leaves the conversation pane.

## Implementation notes

- One edit state (`Shell.rename: Option<{ session, input, problem, retry }>`); the sidebar pencil opens the conversation and the same state.
- The optimistic display changes `Lane.title` and keeps the previous value to restore on failure. A generated title (`set_title_if_empty`, T-578) never overwrites a manual one: the daemon only fills an empty title, and `Event::ConversationTitle { named: true }` from a rename always replaces what a screen shows while `named: false` (a generated topic) only fills an empty one.
- Mockup tokens → app values: surface `--eq-color-raised`→`p.raised`, line `--eq-color-line`→`p.hair`, hover `--eq-color-hover`→`p.wash`, focus `--eq-color-focus`→`p.accent`, error `--eq-color-err(-bg)`→`p.danger()`/`p.danger_bg()`, field background `--eq-color-bg`→`p.code_bg`, primary `--eq-color-action`→`Kind::Primary`, tooltip `--eq-color-inverse`→`p.tip_bg()`. Type 14→`UI`/`SMALL` (the header title stays 14 semibold), 12→`SMALL − 1`.

## As built in T-643 (differences from the above)

- The header pencil keeps its room and only becomes visible on hover: GPUI cannot change layout on hover.
- Characters past 40 are not tinted inside the field; the message and the red count say how many to remove.
- Tab is not trapped inside the card.
- "Renamed: {title}" shows briefly where the window says such things (the composer's note); there is no screen-reader-only live announcement.
