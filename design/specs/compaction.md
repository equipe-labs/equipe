# Showing a conversation being summarized — hand-off spec (T-164)

Status: **settled** — the person chose option B (a flowing bar + elapsed time) (2026-10-30). Pictures: `specs/compaction/options.png` (options compared) · `states.png` (states) · `platforms.png` (per platform) · individual screens `desktop|ios|android-{light,dark}.png`, `tui.png`. The render source is `specs/compaction/build.mjs` (it reads tokens only from `build/css/tokens.css`).

## 1. What it tells
When the engine (Claude Code·Codex) compacts a conversation by itself, or the person presses `/compact` ("Summarize and continue fresh"), the member doesn't answer for a while. Meanwhile it shows that it's **summarizing**, not stuck. The engine gives no progress → **no made-up %** (option B: a flowing bar + the real elapsed time).

## 2. States
| # | State | Starts when | What shows | Ends when |
|---|---|---|---|---|
| 1 | Summarizing (auto) | the engine's compaction-start event | band `Summarizing the conversation` + flowing bar + elapsed | the done/failed event |
| 2 | Summarizing (`/compact`) | the person runs it | only the title: `Summarizing to start fresh` | 〃 |
| 3 | Taking long | 60 seconds of summarizing | title `Summarizing is taking a while`, bar·elapsed as they were | 〃. **No time limit turns it into a failure** |
| 4 | Input while summarizing | the person sends a message | the input is kept, and the band gets a chip `N messages waiting · I'll send them when the summary is done` | delivered in order when it ends (backend). Delivered even on failure |
| 5 | Done | the done event | the band changes to `Summarized / You can keep chatting` (ok, ✓) and goes after 4 seconds. A divider `Summarized · you can keep chatting` stays in the conversation | — |
| 6 | Failed | the failed event | band `Couldn't summarize / The conversation is unchanged. Try again?` + `Try again` (secondary button). Stays until the person closes it | try again → 1 |

Assumptions (unconfirmed — backend to confirm): the engine gives start·done·failed events / on failure the conversation's original text is unchanged / waiting messages are delivered in order.

## 3. Placement
- **Desktop·iOS·Android**: at the very bottom of the conversation, right above the composer. Gap to the composer `space.3` (12). The conversation's scroll area shrinks, and the band stays pinned to the bottom while it shows.
- **Sidebar (member list)**: a summarizing member's row shows the existing "working" spinner + the secondary line `Summarizing · 12s`. Back to its usual state when done.
- **Terminal TUI**: one line right above the input prompt, `⠹ Summarizing… 12s` (+ a secondary line if messages are waiting). Done: `✓ Summarized. You can keep chatting`; failed: `✕ Couldn't summarize. The conversation is unchanged · retry with /compact`.

## 4. Tokens (design system v1)
| Part | Tokens |
|---|---|
| Band fill·border·corners | `color.surface`, `color.line`, `radius.card` |
| Band padding | vertical `space.3` · horizontal `space.4`, gap between elements `space.3` |
| Icon (compress) | 20px (`size.icon.md`); summarizing `color.work` · done `color.ok` · failed `color.err` |
| Title / secondary | `type.small` semibold + `color.text` / `type.meta` + `color.muted` |
| Bar | height 4 (`space.1`), corners `radius.full`, track `color.sunken`, fill `color.work`, segment width 32% |
| Elapsed time | `type.small` + `color.muted`, tabular figures, min width 48 |
| Waiting chip | `type.meta`, fill `color.sunken`, corners `radius.full` |
| Try again button | component `button` secondary (desktop sm) |
| Divider (in the conversation) | line `color.line`, text `type.meta` + `color.muted`, ✓ in `color.ok` |
| Sidebar | the existing member row (`row.*`) + spinner `color.work` |
| Mobile | the band's corners·padding follow the mobile card rule (`radius.card` 16, padding 16) |

**Proposed new token (a source change → T-145 `tokens/`)**: `motion.duration.loop` = 1600ms (one pass of the bar). The spinner's rotation (1000ms) follows the existing sidebar spinner's value. No other new values.

## 5. Motion
- Appearing: opacity 0→1, `motion.duration.fast` (120ms) `out-cubic`, rising `motion.travel.rise` (6px).
- Bar: a 32%-wide segment crosses the track left to right, repeating linearly over `motion.duration.loop` (from the end back to the start).
- Done: icon·title cross-fade `fast`; leaving after 4 seconds is a fade `slow` (200ms).
- **Reduce motion** (web `prefers-reduced-motion`·iOS `accessibilityReduceMotion`·Android animation scale 0·macOS Accessibility): no travel, the bar segment holds still (20% from the left), appearing·leaving only as a `motion.duration.reduced` (100ms) fade. **The elapsed time keeps counting up** (it's text, not motion). The TUI spinner becomes the fixed symbol `…`.

## 6. Copy keys (en shown; every language has its own wording in its locale file, in its register)
| Key | en |
|---|---|
| `compact.title.auto` | Summarizing the conversation |
| `compact.body.auto` | Tidying up a long conversation. You can continue when it's done. |
| `compact.title.manual` | Summarizing to start fresh |
| `compact.body.manual` | Carrying a summary into the next conversation. |
| `compact.title.long` | Summarizing is taking a while |
| `compact.body.long` | The conversation is long and needs more time. Hang tight. |
| `compact.elapsed` | {n}s / {m}m {s}s |
| `compact.queued` | {n} message(s) waiting · I'll send when it's done |
| `compact.done.title` / `.body` | Summarized / You can keep chatting. |
| `compact.divider` | Summarized · you can keep chatting |
| `compact.fail.title` / `.body` | Couldn't summarize / The conversation is unchanged. Try again? |
| `compact.retry` | Try again |
| `compact.sidebar` | Summarizing · {elapsed} |
| `compact.tui.run` / `.queued` / `.fail` | Summarizing… {elapsed} / {n} message(s) will be sent when it's done / Couldn't summarize. The conversation is unchanged · retry with /compact |

The other languages' wording lives with the rest of each front end's words (`crates/desktop/locales/<lang>.json`, and the CLI's). Text uses `word-break: keep-all` + `overflow-wrap: anywhere` (web·desktop), times use tabular figures.

## 7. Accessibility
- **The band is `role="status"` (a polite live region)**. At the start it reads `Summarizing the conversation` once, and **never reads the elapsed time** (keep it outside `aria-live` in a separate element, or `aria-hidden`, so it isn't read every second). Once at 60 seconds, `Summarizing is taking a while`; at the end, `Summarized. You can keep chatting`; a failure once as `assertive`.
- **VoiceOver (iOS)**: group the band into one accessibility element, label = the title, value = `{elapsed}`. Start·done·failed via `UIAccessibility.post(.announcement, …)`. The bar's decoration is hidden. `Try again` has the button trait, press area 44.
- **TalkBack (Android)**: the band is `liveRegion = polite`, the bar has no `contentDescription` (decoration). Done·failed via `announceForAccessibility`. Press area 48.
- Never by colour alone: the compress icon while summarizing, ✓ when done, ✕ + words on failure. Contrast is the `work/ok/err`·`muted` pairs in `docs/contrast.md` (all pass AA).
- The bar isn't progress (a `progressbar` value) → never the `progressbar` role with a value (it would read a value that doesn't exist). If needed, an indeterminate `progressbar` without a value.
- Keyboard (desktop·TUI): a focus ring on `Try again` (`focus.ring-*`). The band never takes input focus.

## 8. Don'ts
- Showing % (a value without grounds) · failing automatically after a set time · blocking the composer while summarizing (input is kept) · the band covering the conversation and hiding messages · emoji·cute wording · bringing in new colours (`work`·`ok`·`err` only).

## 9. What design QA looks at once it's built
The five states summarizing/taking long/done/failed/waiting chip × light·dark × desktop·iOS·Android·TUI, the bar held still under reduce motion, the bar·time staying on one line at 320px wide, how many times the screen reader announces.
