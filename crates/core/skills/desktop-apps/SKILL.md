---
name: desktop-apps
description: Use before you work desktop apps with the `desktop` tools — read an app as a tree, act on refs headless, verify, and when to fall back to screenshots.
---

# Desktop apps

Two kinds of `desktop` tools. Reach for the first kind first.

**The accessibility tree** — `snapshot`, `find`, `act`, `apps`, `windows`, `wait`, `notifications`, `clipboard`.
They read and operate apps through the OS accessibility tree: no pointer, no keystrokes, no focus taken. They don't
wait for the screen, other members can work other apps at the same time, and the person can keep working. A tree costs
a fraction of a screenshot.

**Pixels and the real keyboard** — `screenshot`, `click`/`double_click`/`drag`/`scroll` at coordinates, `type_text`,
`key`, `open_app`, `focus_window`, `elements`. One member at a time, only after the person has left the Mac alone for a
while. Use them for what the tree can't reach: a canvas, a game, an app whose tree is empty, a shortcut (`key`).

## The loop

1. **Look.** `snapshot` with `app`. A small app: that's all. A big one (Slack, Teams, Xcode): `skeleton: true` first,
   then drill into one branch with `root: "e4"` (a line ending in `+26` is a cut branch with 26 children). Know the
   name of what you want? `find` (`role`, `name`) is cheaper than a tree.
2. **Act** on a ref: `act` with `ref` (as printed, `e12`) and `action` — `click` (default), `type` (adds text),
   `set_value` (replaces it), `select` (an option by its text, for menus and pop-ups), `check`/`uncheck` (better than
   `toggle`), `expand`/`collapse`, `scroll`, `scroll_to`, `focus`. `right_click` and `double_click` are physical: they
   wait for the screen.
3. **Verify.** Read the same region again (`snapshot` with the same `root`) or `find` the element: an action that
   says "delivered unverified" didn't prove anything. When the answer says a menu, sheet or alert is open, snapshot
   it with `surface`.
4. **Wait** with `wait` (an element, a window by title, text, an event like `window-opened`), never a fixed sleep.

## When it fails

- `STALE_REF` or `SNAPSHOT_NOT_FOUND`: the UI changed. Snapshot again and use the new refs. Refs last for one snapshot.
- Retry only when the answer says **safe to retry** (nothing was delivered). "It may have landed": read the state
  first — a second click can send twice, a second toggle undoes the first.
- `AMBIGUOUS_TARGET`: two identical controls. Act on one that is unique, or snapshot again with a longer `timeout_ms`.
- `POLICY_DENIED`, or an app that ignores a headless click: `act` with `headed: true` uses the real pointer — it waits
  for the screen and for the person, like the pixel tools.
- A thin tree from an Electron or Chromium app (Slack, Teams, VS Code): snapshot again with `timeout_ms: 15000`; if it
  stays thin, fall back to `screenshot` and coordinates.
- `apps` with `action: launch` opens an app in the background; `activate: true` brings it forward (takes the screen).

## What equipe never lets through

- **Sending in the person's name** — Send, Return in a chat, Reply all, Join, Share, in a messaging or mail app — waits
  for the person's yes, every time, whatever you used (a ref action, a key, a click). Give `why`: what you send, to
  whom, in a line; without it the call comes back to you unasked. Writing the draft goes ahead.
- **Payment, deleting an account, passwords, security settings**, password managers and password fields take no input
  from a member at all. Ask the person to do it.
- **equipe's own windows** are the person's.
- **The Mac's system-wide settings** — VoiceOver (⌘F5), Zoom (⌥⌘8), the Accessibility Shortcuts panel (⌥⌘F5), display and
  keyboard switches, System Settings (you may read it, never click or type in it), VoiceOver Utility. Those shortcuts and
  apps are refused. To check a screen reader, read the browser's accessibility tree (Playwright's ARIA snapshot, axe,
  Chrome's accessibility snapshot) or use VoiceOver inside the iOS Simulator; otherwise ask the person.

## Private by default

Snapshots, the clipboard and notifications show the person's own content. Use what you need for the task, never paste
it into tickets, commits, logs or anything that leaves the Mac, and don't read apps the task doesn't need. Secrets you
type (`act` type/set_value, `clipboard` set) go in on stdin and never show in a process list or a log — but don't type
a password at all; the gate refuses password fields.
