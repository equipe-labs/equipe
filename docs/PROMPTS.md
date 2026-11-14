# Banners, cards and system prompts

Everything equipe can put in front of the person on its own, when, and how it
goes away for good. The rules, then the table.

## Rules

1. A banner shows only when work is blocked and the person can unblock it with
   one action.
2. It never shows for what resolves itself: installing, switching hosts, a
   fresh restart, an update in progress. A locked screen is not one of
   them once a member is held by it: only the person can unlock it (see the
   table).
3. Once put away it stays away until the condition changes in a way the
   person must act on again. Dismissals are kept in the daemon's home
   (`<home>/desktop/setup.json`, per card and identity such as `grant:app` or
   `failed:download`), never in memory, so a daemon restart or an app update
   doesn't bring a card back.
4. A banner about the daemon's Mac says which Mac when that isn't the one in
   front of the person, and then has no button that would open something on
   the wrong Mac: it says where to do it.
5. Background work never makes macOS ask anything. Only the person's click
   does, once, through the prompt budget
   (`crates/desktop-control/src/prompts.rs`: one outstanding prompt per kind,
   the answer remembered per identity in `<home>/prompts.json`, a yes never
   asked again unless macOS takes it back, a no waits 24 hours for anything
   that isn't the person's own click).

## The table

| What | Where | Shows when | Actionable | Goes away for good |
|---|---|---|---|---|
| Grant card ("Turn on equipe — one switch") | desktop strip, `views/desktop_setup.rs`; daemon `desktop/auto.rs` | Accessibility is off **and** a member's desktop call was refused for it, or this is the first set-up and the person hasn't skipped it; not on a locked screen; not if put away for this identity | Yes: on the daemon's Mac "Open System Settings" (the click asks macOS and opens the pane); on another Mac, a text naming that Mac, no button | The switch is seen on; or "Not now" (kept for this identity until the switch has been seen on and was taken back while a member needs it) |
| Permission card ("Turn on Screen Recording for equipe") | the waiting ticket's card, `views/yourturn.rs`; daemon `permission_chore.rs` | A member put the ticket down with `block_on_person`'s `permission` (or steps about macOS privacy panes) **and** equipe's desktop tools don't hold that permission. If they do, the hand-off is refused to the member in one line and the person never sees it. Never several steps | Yes: on the daemon's Mac one primary button (the click asks macOS and opens System Settings at the exact pane) and "Reveal equipe in Finder"; on another Mac, a text naming that Mac, no button | "Done", "Do it for me" or "Later" like any ask; the member goes on by itself once the permission is on |
| Locked-screen card ("<Mac> is locked — a member needs to click there") | desktop strip, `views/desktop_setup.rs` + `unlock.rs`; daemon `desktop/auto.rs` (`lock_card`), `desktop/mod.rs` (`Desktop::locked_waiting`) | The Mac's screen is locked **and** a member's click or typing was refused for it and hasn't gone through since (a member that only reads, or a lock nobody waits on, shows nothing); not if put away for this lock | Daemon on another Mac: "Unlock it" opens Screen Sharing (`vnc://<the daemon's address>`) on the Mac in front of the person, who types the password there: equipe never sees, stores or sends it and never types at a lock screen. If port 5900 refuses the connection: how to turn Screen Sharing on there once (System Settings › General › Sharing), "Copy steps", "Check again". Daemon on this Mac: "This Mac is locked", no button (unlock at the keyboard, Touch ID or Apple Watch). The Status screen's blocked rows say the same ("Mac locked — Unlock") | The daemon sees the screen unlocked (checked every 4 s while the card is up; members resume by themselves); or "Not now" (kept as `locked` in `<home>/desktop/setup.json` until the screen has been seen unlocked) |
| Failed card ("Desktop apps couldn't be set up") | same | The engine or helper set-up failed **and** a member is blocked on it, or it has failed for 10 minutes | Yes: "Try again" (works for a remote daemon too: it runs there) | The next try works; or "Not now" for that failure reason; "Try again" tells it afresh |
| Installing strip | removed | It resolved itself; never a banner. (The first-run setup screen shows its own progress.) | no | n/a |
| "Desktop control now runs as equipe" (old helper row) | removed | Was shown after equipe.app took over from the old helper; asked for pointless clean-up. The stale Accessibility row is harmless | no | n/a |
| macOS Accessibility prompt | `helper::REQUEST` from the card's button, or `equipe desktop setup` | Only on the person's click. The daemon's background loop and the helper's start-up no longer ask | n/a | Once granted: never again for that identity |
| macOS Screen Recording prompt | person's click on "Allow" in Settings › Desktop apps | Only on the person's Allow | n/a | Granted |
| Keychain password prompt | `crates/core/src/keychain.rs` | Only through the budget; background work waits 24 hours after a no | Person | Granted ("Always Allow") |
| Settings › Desktop apps status | `views/settings.rs` | The person opened it | Buttons per line | Not a banner |

Edge cases covered by tests (`crates/core/src/desktop/auto.rs`):

- dismissed, then the daemon restarts: still dismissed;
- an update changes the code signature (macOS forgets the grant): no card until
  work needs it, one card then, and never again once put away;
- helper to app take-over: nobody is told; a card for another identity is a
  different question;
- two screens on one daemon: the daemon tells both, one "Not now" clears both;
- the grant arrives while the card is up: the next look (4 seconds) removes it;
- the person says no: nothing is asked again, the one card stays;
- a locked screen nobody waits on: no card; a member held by it: the lock card
  (remote or local words), gone when the screen is seen unlocked, and a "Not
  now" stays until the next lock;
- a daemon on another Mac: `grant_words` in `views/desktop_setup.rs` names it and
  has no button.

## Members never change the host Mac's system settings

Refused by `equipe_desktop_control::system_guard` for the desktop tools (⌘F5,
fn⌘F5, ⌥⌘F5, ⌥⌘8 and the other Zoom and contrast shortcuts; launching or
quitting VoiceOver or VoiceOver Utility; opening System Settings at
Accessibility; clicking or typing in System Settings), and by
`equipe_core::guard` for Claude Code members' shell commands (`osascript`
targeting VoiceOver, `launchctl`/`open`/`killall` on VoiceOver,
`defaults write` on `com.apple.universalaccess`, `com.apple.VoiceOver4`, the
global domain, `tccutil`). Codex members have no hook, so every member's
prompt says it too, and QA's and the designer's role text names the
alternatives: the browser's accessibility tree, axe, or the iOS Simulator.
