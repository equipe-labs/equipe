# T-617 · Document focus view implementation contract

Chosen: B, return first. Don't duplicate the existing `Doc` surface or its `ScrollHandle`; only change where the same surface is laid out, from the layout root.

## States and transitions

- `document.focus = off | on { origin, previous_layout, previous_focus }`.
- `origin`: `conversation { title }` or `board { title, selected_ticket }`. The header reads `← Back to {title} conversation` / `← Back to {title} board` respectively, from the locale files in every language. The destination doesn't change while focus view is on. Never use only the generic names `PM conversation` or `Board`.
- Enter: the `Focus view` button in the default document header, or the shortcut `⇧⌘F`. Save the document identifier, scroll handle, current selection, split width and keyboard focus, then switch to the focus state. Don't touch macOS full screen.
- Exit: the header's return button, `Esc` and `⇧⌘F` all call the same toggle command. Restore the split width, selected document, scroll position, selected ticket, and keyboard focus on the entry button.
- `Esc` priority: modal → popover/More menu → selection or search inside the document → focus view → the app's existing layers. Close one layer at a time.
- Close document: an ordinary command inside the More menu. Closing during focus also leaves focus, returning to the focus the original context had before the document was opened. Switching documents keeps focus with the new document's identifier, starting from the new document's saved scroll or 0. If the document disappears, leave focus.

## Header

- The focus state always has two rows. Row 1: the return button on the left, document tools on the right. Row 2: the file icon, the full file name and a short folder path.
- Row 1: `space.3` inline padding, `size.row` minimum height. The return button is `type.small`, semibold, `color.text`; `Esc` is `type.meta`, `color.muted`.
- Row 2: `space.4` inline and `space.2` block padding. The file name is `type.small` semibold; the path is `type.meta`/`color.muted`.
- At 760pt or 150%, Reload, Open in Finder, Copy full path and Close document fold into a single `More` menu. The return button and More always stay in the first row.
- The file name gets no two-line limit and no ellipsis. Scripts that break between words (Korean, for one) use `word-break: keep-all`, with `overflow-wrap: anywhere` only for long paths and Latin strings. The header grows taller and the body scrolls below it. The full path is also given in the hover tooltip and the accessible description.
- The body is centred at a maximum width of `layout.measure` (680pt), keeping `layout.gutter` on both sides at 760pt. At 150% the gutter's visual size doesn't grow. The document title is `type.headline`, section titles `type.title`, body `type.ui`, keeping the hierarchy at every width and zoom. Markdown and Mermaid use the existing renderers as they are. Card-style Mermaid switches nodes and arrows to a vertical flow at 1200pt and below (the supported range) so nodes keep a minimum readable width, measured on the document surface's width, not the viewport's. Only node titles are semibold; descriptions are regular, and text in scripts like Korean uses `word-break: keep-all` so no line ends with a single orphaned character.

## Default document header

- The file name and path may take two rows with the tools. Show `Focus view ⇧⌘F` apart from Close document.
- At 760pt/150%, fold Reload, Open and Copy into More, leaving only Focus view and Close document. The two actions have distinct names, `Focus view` and `Close document`.

## Motion

- Only the width and position of the same surface change, with `motion.duration.slide` 180ms / `motion.easing.soft`. No fade, scale or decorative effect.
- Under Reduce Motion, relayout instantly. The scroll position is never animated.

## Accessibility

- Accessible names: `Back to {title} conversation, Escape`, `Back to {title} board, Escape`, `Focus view, Shift Command F`, `More document actions`, `Close document`.
- Right after entering focus, keyboard focus goes to the return button. Focus order: return → Reload → Open file → More → document body. At 760pt/150%: return → More → body.
- More has `aria-expanded`; when open, the header alone shows `Esc closes menu` and the first menu item takes focus. The menu opens below the two-row header so it doesn't cover the file name. Closing it with `Esc` returns focus to the More button; after leaving focus view, to the entry button or the original selection.
- Contrast (existing calculation in `design/docs/contrast.md`): body/background 16.57:1 light, 16.90:1 dark; muted/background 7.25:1 light, 8.03:1 dark; focus/background 6.14:1 light, 9.25:1 dark. All at WCAG AA or above.

## Copy

English below; every other language is translated in the desktop's locale files.

| Situation | English |
|---|---|
| Enter | Focus view |
| Return to conversation | Back to {conversation title} conversation |
| Return to board | Back to {board title} board |
| Key hint | Esc |
| Key hint with the menu open | Esc closes menu |
| More | More document actions |
| Close | Close document |

## QA contract

- At 760/1200pt × light/dark × 100/150% × en and a CJK locale (Korean, as in the `-ko` prototypes), the file name, return button, More, the start of the body and Mermaid are never cut off horizontally.
- Verify entry from a conversation and from the board separately. Compare the same paragraph, split width, selected document/ticket and focus target before and after focus.
- With More open, the first `Esc` closes only the menu and returns focus to More; the second `Esc` leaves focus view and returns focus to the original entry button. The UI map's `focus-menu` → `focus` → `focus-restored` fixes this order.
- Closing or switching documents during focus, and entering or leaving macOS full screen, never leaves an empty surface or an orphaned focus state.
