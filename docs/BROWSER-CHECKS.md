# "Verify you are human" checks in the browser panel

The report: in the browser panel, on dash.cloudflare.com's "보안 확인 수행 중 /
사람인지 확인하십시오" (Cloudflare Turnstile, reached from the OAuth page
`wrangler login` opened in the PM's browser), the person clicks the checkbox
and the check never passes. It comes back every time. A person is there and
clicked, so the click should count.

The goal here is that **the person's own click completes the check**. equipe
does not solve or get around checks, does not make its members' automation
look like a person, and does not hide that the browser is driven over
DevTools. Every experiment below runs on local pages that copy the browser
conditions involved. No real Cloudflare or Turnstile endpoint was contacted.

## How the panel shows a member's page

`crates/core/assets/browser-host.cjs` runs one Chrome. Members' tabs open
**behind a placeholder tab in one minimized "parking" window**, which keeps
them out of the person's way. The panel shows a tab through
`Page.startScreencast` and sends the person's mouse and keys back with
`Input.dispatchMouseEvent` / `dispatchKeyEvent`. A page the person drives is
laid out at the panel's box with `Emulation.setDeviceMetricsOverride`.

## The lab

`scripts/browser-check-lab.cjs` drives the host the way the daemon and the
panel do. A member's lane navigates with its tools, the panel watches and fits
the page, the person takes it (직접 하기), moves the pointer over the picture,
clicks, then hands it back. The page is a stand-in wall on `127.0.0.1` holding
a **cross-site iframe** from `localhost`, which Chrome runs out of process the
way it runs Cloudflare's challenge frame. Both frames report to the lab what
they observe: visibility, focus, rAF and timer rates, IntersectionObserver,
lifecycle events, every pointer and mouse event with its fields, their window
and screen geometry, a third-party cookie, and what automation they can see.

    npx -y -p playwright@1.64.0 node scripts/browser-check-lab.cjs [host.cjs]

It needs Chrome and a desktop session, so it isn't run in CI. The smaller
probes (window states, parking and staging over time) were throwaway scripts
built the same way.

## What it showed

Mac with a 2560×1440 display (dpr 2), Chrome stable, playwright 1.64.0.

### Not the cause: visibility, drawing, timers, focus, cookies

The first suspicion was that a tab in a minimized window is hidden, so a
widget waiting for visibility or animation frames would never finish. It is
not hidden. A member's parked tab, watched or not, reports
`visibilityState: 'visible'` in the top frame and in the cross-site frame. It
runs `requestAnimationFrame` at 60/s and a 20 ms timer at about 50/s, its
IntersectionObserver fires, and no `freeze` happens. Playwright already
launches Chrome with `--disable-background-timer-throttling`,
`--disable-backgrounding-occluded-windows` and `--disable-renderer-backgrounding`,
and turns on focus emulation for every page. The same held over 30–40 s runs in
both window states.

Some runs did show stalls, with rAF at 0–20/s and timers at 1–8/s, but in
both window states alike. They happened only while the machine's load average
was 150–400 (parallel builds). At a load of about 40 the same probe ran flat at
60/50 in every state. Starved CPU throttles all pages; parking does not.

| | parked tab | real window |
|---|---|---|
| visibility (top / challenge frame) | visible / visible | visible / visible |
| rAF/s, timer/s (load ≈ 40) | 60, 50 | 60, 50 |
| `document.hasFocus()` top | true (emulated) | true |
| third-party cookie in the frame | set and read back | set and read back |

The challenge frame's `hasFocus()` is `false` until the click and `true` at
the click. A real browser behaves the same: focus moves into the frame on
mousedown.

### Delivered correctly already

The person's click reaches the cross-site frame trusted (`isTrusted`), as
`pointerType: 'mouse'`, at exactly the point clicked: (116, 36) in the frame
for a click aimed at (116, 36). It arrives after `pointerover / pointerenter /
mouseover / mouseenter / pointermove` from the moves the panel forwards.
Coordinate mapping (`stream::to_page`, letterboxing, device scale) was exact.

### The cause: the click comes from nowhere on any screen, with no pressure

What a parked tab tells the challenge frame, against the same page in a real
window:

| read in the challenge frame | parked, the person driving it (before) | in a real window |
|---|---|---|
| `window.screenY` | **1440**, the foot of a 1440-high screen | 52 |
| `outerHeight - innerHeight` | **0**, no toolbar | 87 |
| click `screenY` | **1636**, below the screen (`screen.height` 1440) | 335 |
| `pointerdown.pressure` (button down) | **0** | 0 from DevTools, 0.5 from a real mouse |
| `outerWidth × outerHeight` (top) | 560×700, the emulated box itself | the window's |
| `screen.availHeight` top vs. frame | **1440 vs. 1410** (emulation drops the menu bar in one frame only) | 1410 / 1410 |

A tab that has never been in front of a window has no place on the screen.
Chrome reports it at the foot of the display with no browser chrome. So every
click the person makes through the panel has screen coordinates below the
bottom of the screen, and it presses with zero pressure, which the Pointer
Events spec reserves for no button down (a mouse with a button down reports
0.5). Emulating the panel's box on top of that gives a window whose outer size
is its viewport, and makes the top frame and the challenge frame disagree
about the screen. No person's mouse produces any of these. They are exactly
what a challenge reading the click looks at to decide whether a person made
it. The person's real click arrives looking like no person's, so the check
refuses it and asks again: the endless loop.

This is equipe's doing. The parking design gives a page a fictional place,
and the emulation adds to it. The person's click and the mapping of the
coordinates are both fine.

## The fix

All in the browser host's visibility and lifecycle handling and the input path:

1. **The stage** (`restage`, `putOnStage`, `sizeStage` in browser-host.cjs).
   While the person drives a page the panel shows, that tab comes to the front
   of a real, normal (not minimized) window. "Drives" means a member's page in
   their hands (직접 하기, a check equipe handed them, or their click or key),
   or their own tab once they put a hand on it. The window is sized so the
   page's viewport is the panel's box, so it is laid out the same with no
   device-metrics emulation. It is brought up behind the app the person is in
   (`giveBack`), so they keep their keyboard. Given back, the placeholder goes
   in front and the window goes down again (`restack`, now one call at a time,
   so a take and a give-back in quick succession can't leave it half done).
   The screencast keeps flowing through cross-site navigations in both states.
   After the fix the lab reads `screenY` 52, a toolbar of 87 px, the click at
   `screenY` 335 within the 1440 screen, a 560×700 viewport, and the stand-in
   check passes.
2. **Pressure** (`input` in browser-host.cjs). The person's mouse goes as a
   mouse goes: `pointerType: 'mouse'`, `force` 0.5 while a button is down and
   0 otherwise (`pointerdown.pressure` reads 0.5). This is only for the
   person's input. Members' clicks are Playwright's and are untouched.
3. **The pointer** (`browse::Pointer`, used by `views/browser.rs`). Moves are
   paced at 60 a second, and the last one is never dropped: it goes once its
   time comes. Before a press there is always a move to the press's own point
   if the page last saw the pointer elsewhere, as a real mouse always moves to
   where it presses. Picking another tab or leaving the picture starts it
   over. Tested in `browse.rs`.
4. **Moves never take control.** A pointer passing over a member's picture
   used to take the member's browser, because the host took control on any
   input. Now only a press or a key takes it. Before that, moves and the wheel
   don't reach the page. The press that takes it is preceded by a move to its
   point (the host sends that one, since the earlier moves weren't
   forwarded). Moves don't count as a hand on the page and don't reset the
   idle hand-back. The panel sends moves only while the person drives the tab
   on screen. The smoke test checks this (`scripts/browser-host-smoke.cjs`).

## Limits, and the way out

The fixes make the page and the person's click honest. They don't make the
browser something it isn't. The challenge frame can still see that DevTools
drives this Chrome. Playwright's bindings are on its window
(`__playwright__binding__`, `__playwright__binding__controller__`, and
equipe's `__equipeWho`). Every page has a DevTools session, and the person's
click is still dispatched through DevTools: `pressure` and `screenX` come from
it, not from the OS. The `--disable-blink-features=AutomationControlled`
switch that predates this work hides `navigator.webdriver`, so the lab reads
`false` there. A console-serialisation probe for DevTools reads `false` too.
equipe hides none of the rest and adds nothing to hide it. A check that
refuses any DevTools-driven browser will still refuse this one.

For that case the banner now has a way out. On a check the person answers by
clicking (Challenge or Captcha), it offers **"이 Mac의 Chrome 창에서 열기"**
alongside 직접 하기, and again in the "on it" line once the person is on it.
It brings the member's own Chrome window up on the daemon's Mac, un-minimized,
in front and focused, with that member's tab. The person then clicks the check
there with their own mouse: real OS input and a real window. It is the same
browser with the same sign-ins, and nothing is copied between browsers. When
the panel is attached to a daemon on another Mac (SSH), the button says
"{host}의 Chrome 창 띄우기", and the banner says plainly that the browser runs
on that Mac. Use it at that Mac or over screen sharing, because the window
comes up there, not here. Its place is then left to Chrome: this window's
position means nothing on the other Mac.

Not done, on purpose: no fingerprint or flag changes, no removal of
Playwright's bindings from third-party frames, no automatic solving or
retrying, and no moving cookies to another browser. For an OAuth step like
`wrangler login`, whose callback goes to `localhost` on the daemon's Mac, the
person can also open the printed URL in their own browser on that Mac and sign
in there themselves. Nothing of the member's browser is needed for that.
