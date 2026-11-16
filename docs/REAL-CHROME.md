# Real Chrome: members in the person's own browser

> **Status:** prototype, off by default (`equipe desktop chrome on`, per squad).
> Built on [desktop control](./DESKTOP-CONTROL.md): same helper, same
> Accessibility grant, same gates. macOS first.

## Why

Members browse in a Chrome that Playwright drives (`core/src/browser.rs`,
`assets/browser-host.cjs`). Sites can tell: Cloudflare checks fail even when the
person clicks them, sign-ins have to be copied between browsers and get
rejected, Google reports a "new device", and Workspace policies force separate
profiles. The person's own Chrome has none of these problems. It holds their
real profiles and sign-ins, and it has no automation switch or debugging port.

**Decision:** for web work that needs the person's accounts, members use the
person's real Chrome, driven through the macOS Accessibility API. That is
the same way they already use desktop apps. There is no browser extension
(no extra install step) and no CDP. Playwright stays for headless development
work, where automation is the point (tests, screenshots of local builds, design
QA).

## The engine: agent-desktop, already in equipe

equipe already depends on
[agent-desktop](https://github.com/vercel-labs/agent-desktop) (Vercel Labs,
Apache-2.0, compatible with equipe's license and listed in
THIRD_PARTY_NOTICES.md). It is pinned at 1.2.0 and hash-checked, and it runs as
a child of the desktop helper under the helper's single Accessibility grant
(see [DESKTOP-CONTROL.md](./DESKTOP-CONTROL.md#the-integration-choice-the-cli-run-by-the-helper)).
Real Chrome reuses it unchanged:

- **Chromium's web tree.** agent-desktop turns on Chrome's renderer
  accessibility itself. It sets `AXEnhancedUserInterface` (or
  `AXManualAccessibility`) on the app element, reads the flag back, and waits
  out Chromium's two-second debounce
  ([upstream notes](https://github.com/vercel-labs/agent-desktop/blob/main/docs/solutions/runtime-errors/chromium-readable-tree-with-inactive-ax-actions.md)).
  equipe adds no FFI of its own for this.
- **Refs, headless actions, sessions.** Each member has its own session, and
  agent-desktop re-identifies an element before acting (`STALE_REF`). Actions
  go through AX (`AXPress`, `AXValue`, select, scroll-to) with no pointer and
  no focus taken, so members work their own windows in parallel.
- **Window ids are window numbers.** `w-4521` is `w-<kCGWindowNumber>`
  (upstream `cg_window.rs`), so the same id addresses ScreenCaptureKit and
  `_AXUIElementGetWindow`.
- **Not taken:** the C library, for the reasons already given (no window
  targeting in `ad_snapshot`, and a crash would take the helper down). Also not
  taken: a newer pin just for this, because 1.3.x adds the VM sandbox and fixes
  but nothing Chrome needs. agent-desktop's cursor overlay (one per agent) is
  not used: headless actions move no pointer.

Maturity: agent-desktop is young (1.0 at the end of October 2026, 1.3.1 on
2026-11-13) but well tested upstream, and equipe already runs it in production
for desktop apps. The pin, hash check and per-call process limit the damage a
bad release can do to one failed call.

## How a member works a page

| Tool (`mcp__desktop__…`) | What | Screen |
|---|---|---|
| `chrome_profiles` | the person's profiles: folder, name, Google account (masked `j***@example.com`), and the squad's profile per site | none |
| `chrome_open` | a new window of one profile at a URL | brings Chrome forward once, so it takes the lease |
| `chrome_snapshot` | that window's page as compact refs (`skeleton`, `root`, `all`) | none (headless) |
| `chrome_act` | click / type / set_value / select / check / scroll… by ref | none (headless) |
| `chrome_read` | the page's text in reading order (`find`, `max_chars`) | none |
| `chrome_capture` | a picture of that window only | none (Screen Recording) |
| `chrome_close` | close that window (its close button, `AXPress`) | none |

1. **Open.** `open -n -g -b com.google.Chrome --args --profile-directory=<P>
   --new-window <url>`. `-n` makes macOS start a launcher even when Chrome is
   running. Chrome's single-instance check then hands the URL to the running
   browser, so the window gets the person's real profile and sign-ins, and the
   launcher exits. No AppleScript is used, so there is no Automation prompt.
   Only `http(s)` URLs open (never `chrome://`, `file://`, `javascript:` or
   `data:`). equipe lists Chrome's windows before and after, and the new one
   becomes the member's. Opens run one at a time, so each member sees only its
   own new window.
2. **Own windows.** The helper keeps member → windows. Every chrome tool takes
   only the member's own windows. Refs from another member's snapshot are
   unknown to a member (agent-desktop sessions are per member). While real
   Chrome is on for a squad, the generic desktop tools refuse Chrome: a
   `snapshot`/`find` of it, or an `act` on a Chrome ref. The person's own
   windows stay theirs. Several members each keep their own windows and work
   at the same time.
3. **Read** with `chrome_snapshot` (default timeout 8 s, which leaves room for
   the renderer's switch) or `chrome_read`. A page that shows a human check is
   flagged in the answer.
4. **Act** by ref, headless. Keystrokes through `CGEventPostToPid` are not
   offered: only accessibility actions. A page that needs real keys is a
   follow-up (see Limits).
5. **Wait** for navigation with the existing `wait` tool (`app: "Google
   Chrome"`, `text`, or `event: window-…`), then snapshot again.
6. **Look** with `chrome_capture`, which uses ScreenCaptureKit on the window
   number. It never captures the rest of the screen.
7. **Close** when done. A window the person closes is forgotten the next time
   it is used.

**Downloads, uploads, dialogs.** Downloads land in the person's Downloads
folder as usual (Chrome's own setting). Upload and Save sheets are AX sheets of
the window: `snapshot` with `surface: sheet`, then set the path field by ref.
That works in the prototype through the generic tools, and it is a follow-up
to add it to `chrome_snapshot`. JavaScript `alert`/`confirm` dialogs are AX
dialogs in the same window. Permission prompts (camera, location,
notifications) are the person's to answer.

## Which profile, remembered

`chrome_profiles` reads only Chrome's `Local State` profile list
(`profile.info_cache`: folder, name, account address). It never reads cookies,
passwords, history or the Keychain, and addresses are masked before any member
sees them. When a site's profile isn't plain, the member asks the person
(`ask_decision`, "which account for github.com?"). It then opens with
`profile` and `remember: true`. The choice is kept per squad and per site
(`mail.google.com` and `accounts.google.com` count as one site) in
`~/.equipe/desktop/chrome-profiles.json`, so the person is asked once. A
member can still write the fact into squad memory for context ("GitHub work
is in the Work profile").

## The person's part: sign-ins and human checks

A sign-in or a human check in a member's window is the person's to do. When a
member with a real window calls `show_my_browser`, the daemon (gateway, before
the runtime; `core/src/real_chrome.rs`) answers that the page is in its own
window of the person's Chrome. The member then calls `block_on_person` naming
that window (title, profile). The person signs in or passes the check in their
own browser. Nothing is copied back, because the sign-in is already where it
belongs. Without a real window, or with the feature off, the existing
hand-off runs unchanged (`window_handoff.rs`, `signin*.rs`).

**In-app view (follow-up, design).** The app shows the member's window live
and forwards the person's clicks to it as real input:

- Stream: ScreenCaptureKit `SCStream` filtered to the one window number
  (`SCContentFilter(desktopIndependentWindow:)`), 10–15 fps, sent to the app
  over the existing helper → daemon → app path as JPEG frames. The window
  keeps rendering even when another window covers it.
- Input: the app maps a click in the view to window coordinates, and the
  helper posts it with `CGEventPostToPid` to Chrome's pid (it reaches the
  window without the pointer, the same way remote-desktop tools do it). Text
  is typed the same way. This is the person's own input, decided by the
  person, so it is never a member action. It doesn't count against the
  member's lease.
- Or "Show the window": bring that window forward (`focus-window`) for a
  person at the Mac. That is the simplest path, and the default when the app
  is on the same Mac.

## Permissions, asked once

- **Accessibility for "equipe"** is the same single switch that desktop apps
  need. No new entry appears.
- **Screen Recording** is needed only for `chrome_capture` and the in-app
  view. Until it is on, members read the tree, and `chrome_capture` says so.
- Both are asked through the existing guided card (Settings › Squad › Desktop
  apps, `equipe desktop setup`) and the prompt budget
  (`desktop_control::prompts`): one macOS prompt of each kind, remembered.
- No Automation (AppleScript) permission, no extension, nothing installed in
  Chrome.

## Safety

- **Human checks are the person's.** A ref whose name or description marks a
  CAPTCHA, reCAPTCHA, hCaptcha, Turnstile or "verify you are human" is
  refused before anything runs. So is every input on a challenge page ("Just a
  moment…"). The member is told to call `show_my_browser`. Nothing evades bot
  detection: the browser is genuinely the person's, and checks are passed by
  the person.
- **Payment, password and security pages take no input.** The send gate
  (`desktop_gate`, unchanged rules) judges every `chrome_act` by the page
  title and the element. Password fields are `AXSecureTextField`. Their values
  never show in snapshots or `chrome_read`.
- **Sending in the person's name asks.** A web messaging page (Gmail, Outlook,
  Slack, Teams, Discord, WhatsApp, Telegram, Google Messages, recognised by
  the window title) counts as the desktop app of the same name. A ref click
  on Send there waits for the person's yes, with the draft. Approvals are
  never decided by an agent.
- **Action log.** Every chrome call is a JSON line in
  `~/.equipe/desktop/chrome-actions.log` (time, member, tool, action, window,
  site, ok). It never holds typed text, page text or full URLs. It is rotated
  at 1 MB.
- **Per squad on/off.** `equipe desktop chrome on|off [--squad]`, off by
  default, set by the person only. Members also need the desktop to be on for
  them (it is by default once Accessibility is on).
  `equipe desktop off <member>` takes it away from one member.
- Text a member types goes to agent-desktop on stdin, never in process
  arguments.

## Coexisting with Playwright

| Need | Path |
|---|---|
| The person's accounts, sites with bot checks, Google/Workspace | real Chrome |
| Local dev servers, tests, design QA, recorded flows, headless batch work | Playwright (unchanged) |

Both stay available. A member picks real Chrome when the work needs the
person's sign-in. `show_my_browser` routes by whether the member has a real
window. A later step can stop copying cookies into the Playwright profile for
sites already handled in real Chrome.

## Limits

- **Canvas apps** (Figma, Google Sheets' grid, maps, games) have thin trees.
  The member reads what it can, uses `chrome_capture` to look, and the person
  does the rest. Physical keys through `CGEventPostToPid` are the follow-up for
  pages that need them.
- **Locked screen.** Headless actions may run while the screen is locked
  ("Work while the screen is locked"), but Chrome may throttle hidden
  renderers. Opening a window waits for an unlocked screen.
- **Focus once.** Opening a window brings Chrome forward once (so
  `chrome_open` takes the lease). A follow-up can open windows off-screen or
  minimised once renderer throttling is measured.
- **Performance.** A Chrome snapshot costs one agent-desktop process plus the
  AX walk, typically tens to hundreds of milliseconds. A big page wants
  `skeleton` and `root`. The first call on a window pays Chromium's
  two-second activation.
- **Window identity.** If the person opens a Chrome window in the same
  half-second as `chrome_open`, the member could take it. Opens are
  serialised, and the window's title is shown back to the member.
- Human-check detection is word-based (English plus the challenge widgets'
  own titles). Other languages' checks rely on the person-facing card and the
  member's instructions.
- Window ownership lives in the helper's memory and is forgotten when the
  helper restarts. Members then open a window again.

## Smoke test (the owner, on a Mac with Chrome)

1. `equipe desktop status` reports Accessibility on. Then run `equipe desktop
   chrome on` in the squad's folder.
2. Restart the member (its MCP tool list is read at start). It now lists the
   `chrome_*` tools.
3. Ask it to "list my Chrome profiles". You should see masked accounts only.
4. Ask it to open `https://example.com` in a named profile. A new window opens
   in that profile, and your other windows are untouched.
5. "Read the page" (`chrome_read`), then "click More information"
   (`chrome_snapshot` → `chrome_act`), then "close your window".
6. Open a page with a CAPTCHA demo. The member must refuse to click it and
   point you at the window.
7. Check `~/.equipe/desktop/chrome-actions.log`.
8. `equipe desktop chrome off` hides the tools again (after a restart) and
   refuses calls at once.
