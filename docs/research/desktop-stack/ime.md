# GPUI Korean IME and glyph sharpness check (T-53)

Status: **the example and the checklist are done; the check on a real machine is not.** This session had no screen-capture permission and no UI automation permission (key input, IME switching) (`screencapture`: "could not create image from display", `osascript` System Events did not answer), so the scenarios were not run. Every result cell below is **unchecked**; no guess is written down as fact.

## Running it

```
cargo run --release -p equipe-desktop --example ime_probe
PROBE_FONT="Apple SD Gothic Neo" # the default; set it only to try another font
```

The window has a one-line input (`Input`), a multi-line input (`Textarea`, growing from 3 to 8 rows), the same sentence at 13/14/16 pt, and an event log at the bottom (change / ENTER / focus / blur). When the log's `ENTER(send?)` line appears shows by eye whether "an Enter during composition leaks out as a send".

## Scenarios (macOS Korean 2-Set)

The inputs below are Korean, typed with the Korean 2-Set input source.

| # | Scenario | How to check | Result |
|---|---|---|---|
| 1 | Composition underline | While typing Korean "한", the last character is underlined (being composed) | unchecked |
| 2 | Candidate window position (Hanja key / ⌥Enter) | After typing Korean "한", ⌥Enter → does the candidate window open near the caret? | unchecked |
| 3 | Backspace during composition | Korean "한" → ⌫ → "하" → ⌫ → "ㅎ" → ⌫ → empty | unchecked |
| 4 | Mouse click / focus change during composition | While composing Korean "한", click another input or an empty spot → is the character committed, lost or doubled? | unchecked |
| 5 | ⌘C / ⌘A during composition | Works without a panic (the process ending)? | unchecked |
| 6 | Enter: commit vs send | Enter while composing Korean "안녕" → only a commit in the log; `ENTER` on the second Enter | unchecked |
| 7 | First character right after switching English ↔ Korean | Type English → switch with Caps/⌃Space → the first consonant is neither lost nor doubled | unchecked |
| 8 | A long sentence typed fast | 40+ characters, fast → no characters dropped or out of order | unchecked |
| 9 | Cursor movement in mixed Korean and English | ←/→/⌥←/⌥→ in "abc한글def" (Korean in the middle) | unchecked |

## Rendering comparison (not done)

- Capture 13/14/16 pt × 1x/2x beside the same sentence in TextEdit (Apple SD Gothic Neo), and keep them in this folder as `render-*.png`.
- 1x needs an external display, or the "Low resolution" mode in System Settings' display resolution.

## Clues from reading the source (gpui-pre-macos 0.3.8, not checked by running)

- Rendering: `recommended_rendering_mode` in `text_system.rs` is always `Grayscale`; subpixel (LCD) antialiasing is not used. The system's own text has done the same since macOS 10.14, so that alone does not seem to cause blur. It does imitate stroke weight (Core Graphics smoothing) with `glyph_dilation_for_color`, which turns off with `AppleFontSmoothing=0`: if the weight looks different from TextEdit's, check that value first. Glyphs are rasterized into the atlas separately per horizontal subpixel position (`SUBPIXEL_VARIANTS_X`), so if blur shows, a patch cutting those variants to 1 (snapping the position) is a candidate workaround. Not tried yet.
- IME: `handle_key_event` sends a key to `inputContext` first while composing (when `marked_text_range` exists). With the Korean input source on, printable keys also go to the IME first (`is_ime_printable_key`), so problems like #7 are reduced by design.
- The candidate window's position (#2) depends on `first_rect_for_character_range` turning the rectangle it gets from the input handler's `bounds_for_range` into screen coordinates. That conversion is computed differently for `NSFullSizeContentViewWindowMask` windows and others, so the real position has to be seen in the window Root makes (gpui-kit `open_window`). It may be the same symptom as zed #64850.
- Whether the known issues (zed #65453 underline, #64389 panic copying during composition, #46319 Hanja key, #57777 CJK blur) remain in gpui-pre 0.3.8 is for the table above to find out.

## Worked around / not done

Nothing yet (not checked).

## Conclusion

**① Go on with GPUI as it is (the person's decision, 2026-10-14)** — decided without the check on a real machine. If Korean (Hangul) input problems come up, we fix them by patching GPUI Kit/gpui-pre. The table above stays as the checklist to fill in with `ime_probe` when a problem is met. No comparison with Tauri was made.
