# T-617 · Document focus view — implementation contract

Choice: B, return first. Don't duplicate the existing `Doc` surface or its `ScrollHandle`; only the layout root changes where that same surface sits.

## States and transitions

- `document.focus = off | on { origin, previous_layout, previous_focus }`.
- `origin`: `conversation { title }` or `board { title, selected_ticket }`. The header reads `← Back to {title} conversation` / `← Back to {title} board` (Korean: `focus.back_conversation` / `focus.back_board` in `locales/ko.js`). The destination doesn't change while in focus view. Never only the generic `PM conversation` / `Board`.
- Enter: the `Focus view` button in the default document header, shortcut `⇧⌘F`. Save the document id, scroll handle, current selection, split width and keyboard focus, then switch to focus. Don't touch macOS full screen.
- Leave: the header's return button, `Esc` and `⇧⌘F` call the same toggle command. Restore the split width, selected document, scroll position, selected ticket, and keyboard focus on the entry button.
- `Esc` order: modal → popover/More → selection/search inside the document → focus view → the app's existing layers. One layer per press.
- Close document: an ordinary command inside More. Closing while in focus also leaves focus and returns to the focus held before the document opened in its original context. Switching documents keeps focus with the new document id, starting from the new document's saved scroll or 0. If the document disappears, leave focus.

## Header

- In focus the header is always two rows. Row 1: the return button on the left, document tools on the right. Row 2: file icon, full file name, short folder path.
- Row 1: `space.3` inline padding, `size.row` minimum height. Return button `type.small`, semibold, `color.text`; `Esc` in `type.meta`, `color.muted`.
- Row 2: `space.4` inline, `space.2` block padding. File name `type.small` semibold, path `type.meta`/`color.muted`.
- At 760pt or 150%, Reload · Open in Finder · Copy full path · Close document fold into one `More`. The return button and More always stay on the first row.
- The file name has no two-line limit and no ellipsis. Korean uses `word-break: keep-all`, with `overflow-wrap: anywhere` only for long paths and Latin text. The header grows and the body scrolls below it. The full path is also in a hover tooltip and the accessible description.
- The body is centred at `layout.measure` (680pt) max width and keeps `layout.gutter` on both sides at 760pt. At 150% the gutter doesn't grow visually. Document title `type.headline`, section titles `type.title`, body `type.ui` — the hierarchy holds at every width and scale. Markdown and Mermaid use the existing renderers. Card Mermaid switches nodes and arrows to a vertical flow at 1200pt and below (the supported range) so nodes keep a minimum reading width, measured on the document surface's width, not the viewport. Only node titles are semibold; descriptions are regular, and Korean uses `word-break: keep-all` so no line is left with a single syllable.

## Default document header

- File name/path and tools may take two rows. `Focus view ⇧⌘F` is shown apart from Close document.
- At 760pt/150%, Reload, Open and Copy fold into More, leaving only Focus view and Close document — two actions with distinct names, `Focus view` and `Close document`.

## Motion

- Only the same surface's width and position change, with `motion.duration.slide` 180ms / `motion.easing.soft`. No fade, scale or decoration.
- With Reduce Motion, relayout at once. Scroll position is never animated.

## Accessibility

- Accessible names: `Back to {title} conversation, Escape`, `Back to {title} board, Escape`, `Focus view, Shift Command F`, `More document actions`, `Close document` (Korean in `SPEC_KO`, `locales/ko.js`).
- Right after entering focus, keyboard focus is on the return button. Focus order: return → Reload → Open file → More → document body. At 760pt/150%: return → More → body.
- More has `aria-expanded`; while open, the header alone shows `Esc closes menu` and focus moves to the first menu item. The menu opens below the two-row header so it never covers the file name. Closing it with `Esc` returns focus to More; leaving focus returns it to the entry button / the original selection.
- Contrast (existing figures, `design/docs/contrast.md`): body/background 16.57:1 light, 16.90:1 dark; muted/background 7.25:1 light, 8.03:1 dark; focus/background 6.14:1 light, 9.25:1 dark. All WCAG AA or better.

## Copy

English below; each line's Korean is in `SPEC_KO` in `locales/ko.js`.

| Where | English | Key |
|---|---|---|
| Enter | Focus view | `focus.enter` |
| Back to a conversation | Back to {conversation title} conversation | `focus.back_conversation` |
| Back to a board | Back to {board title} board | `focus.back_board` |
| Key hint | Esc | — |
| Key hint, menu open | Esc closes menu | `focus.esc_menu` |
| More | More document actions | `focus.more` |
| Close | Close document | `focus.close` |

## QA contract

- At 760/1200pt × light/dark × 100/150% × ko/en, the file name, return button, More, the start of the body and Mermaid are never clipped horizontally.
- Check entering from a conversation and from a board separately. Before and after focus, compare the same paragraph, split width, selected document/ticket and focus target.
- With More open, the first `Esc` closes only the menu and returns focus to More; the second `Esc` leaves focus view and returns focus to the original entry button. The UI map's `focus-menu` → `focus` → `focus-restored` fixes this order.
- After closing or switching the document in focus, or entering/leaving macOS full screen, there is no empty surface or orphaned focus state.
