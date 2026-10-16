# Desktop control

Members working desktop apps (Microsoft Teams, say) the way they work the
browser: see the screen, click and type, open apps. This is the T-20 spike
(ROADMAP principle 2): before building the tools, learn whether equipe's daemon,
running as a launchd login service, can do any of that — and what it costs the
person to allow it.

Directions already set by the person: equipe builds the tools itself; the daemon
runs in the Mac's login session; one member uses the screen at a time; sending
anything to others in the person's name needs their yes.

Probe: [`research/desktop-control/probe`](./research/desktop-control/probe) (not
product code, not in the workspace). `agent.sh` loads it as a throwaway
LaunchAgent (`im.equipe.probe-t20`) in `gui/<uid>`, the way `equipe daemon
install` loads the daemon, runs it once and unloads it.

## What the probe showed

Mac mini M4 Pro (`Mac16,11`), macOS 26.2, the person logged in at the console
and working over SSH (Tailscale) and Jump Desktop. 2026-10-13/14.

| Question | Result |
|---|---|
| A LaunchAgent bootstrapped into `gui/501` — from an SSH shell — runs in the GUI session | ✅ `launchctl managername` → `Aqua`; on console, login done |
| Its child processes are in the same session | ✅ `Aqua` |
| The daemon running the squad | ❌ at first `Background`: started by `spawn_detached` from an SSH shell. ✅ `Aqua` once the person ran `equipe daemon install` (`im.equipe.daemon` in `gui/501`) |
| A display exists without a monitor | ✅ here, but only because something provides one: display 5, 1472×828 pt at **1×** (Jump Desktop's virtual display, most likely) |
| Window titles of other apps, without Screen Recording | 3 of 25 — the permission gate is real |
| Screen Recording granted (person added the probe with "+") | ✅ `CGPreflightScreenCaptureAccess` true; 24 of 24 window titles readable |
| **Capture from the LaunchAgent** | ✅ ScreenCaptureKit 1568×882 (sized by SCK) in 80–125 ms; `CGDisplayCreateImage` 5120×2880 in 13–30 ms. The image shows real app windows, not just wallpaper |
| macOS 26 asks again on first direct capture | ⚠️ a second dialog appears: *"desktop-probe wants to bypass the system private window picker and directly access your screen"* (Allow / Open System Settings). Capture already worked while it was up |
| Click and type **without** Accessibility | ❌ silently dropped: no error, the pointer doesn't move, nothing typed. TextEdit opened fine (`open -a` needs nothing) |
| A bare executable in Accessibility | ❌ the person added it with "+", it never showed in the list. Screen Recording took it; Accessibility wants an **app bundle** |
| App bundle, fixed bundle id, fixed signature | `bundle.sh`: `~/Applications/equipe Desktop Probe.app`, `im.equipe.desktop-probe`, signed with a self-signed certificate kept in a keychain of its own → designated requirement `identifier "im.equipe.desktop-probe" and certificate leaf = H"9510…"` instead of a cdhash. `request` puts it in both lists by itself |
| Display after the person connected with Jump Desktop | 2560×1440 pt at **2×** (5120×2880 px) — it was 1472×828 @1× earlier: the virtual display changes with the remote client |
| A rebuilt, re-signed probe keeps its grants | ✅ rebuilt twice, re-signed with the same certificate: Screen Recording, Accessibility and Post Event all still true, no new prompt |
| Open TextEdit from the LaunchAgent | ✅ `open -F -a TextEdit <file>` (no permission). ⚠️ it opens **behind** the frontmost app — macOS 14+ cooperative activation doesn't let a background process bring an app forward |
| **First full run: click + type** | ⚠️ the click went to where TextEdit's window was, but the person's **Ghostty terminal** was on top: the text was typed into their shell prompt (no Enter). Clicking at a window's coordinates is not clicking that window |
| Bring the app forward | ✅ through Accessibility: `AXFrontmost = true` on the app, `AXRaise` on the window |
| What a click at (x, y) will reach | Window list (`CGWindowListCopyWindowInfo`) is **wrong** here: BetterDisplay's click-through overlay covers the whole screen. ✅ AX hit test (`AXUIElementCopyElementAtPosition`) answers TextEdit's `AXTextArea` |
| **Click at a point, type text incl. Korean, read it back** | ✅ pointer moved to (503, 299), keyboard focus on TextEdit (`AXFocusedApplication`), typed `hello from equipe probe 안녕하세요` as Unicode events; AX read back `Hello from equipe probe 안녕하세요` (TextEdit auto-capitalised the first letter). Seen on the capture too |
| macOS's "bypass the private window picker" dialog | ⚠️ stays up until the person answers; capture, click and type all worked while it was showing |
| macOS notifications seen during the runs | "Privacy & Security blocked 'equipe' from modifying apps" — **App Management**: a process charged to `equipe` (the squad running under the daemon) rewriting a signed bundle in `~/Applications`. "'equipe' can run in the background" — the login service |

**Answer to the spike: yes.** A process launched by launchd in the person's GUI
session captures the screen, clicks, types (Korean included), opens apps and reads
apps' UI through Accessibility — once the person grants Screen Recording and
Accessibility to a **signed app bundle**, and the grant survives rebuilds when the
signature's identity is fixed.

Reproduce: `bundle.sh <built probe>` → `agent.sh "<app>/Contents/MacOS/desktop-probe" request|status|run <out dir>`.
The probe refuses to click unless the AX hit test lands in TextEdit, and to type
unless TextEdit has keyboard focus. Clean-up after the spike: remove
`equipe Desktop Probe` from both Privacy lists, delete
`~/Applications/equipe Desktop Probe.app` and
`~/Library/Application Support/equipe-probe-t20/` (signing keychain, build, logs).

## Decisions

### APIs

- **Screen: ScreenCaptureKit** (`SCScreenshotManager`, macOS 14+). It renders
  straight to the size the model gets (`SCStreamConfiguration` width/height), so
  there's no resize on our side. `CGDisplayCreateImage` / `CGWindowListCreateImage`
  are deprecated (obsoleted in the macOS 15 SDK) — kept in the probe only as a
  comparison, not for product use.
- **Input: CGEvent** posted at the HID tap. Text goes in as Unicode
  (`CGEventKeyboardSetUnicodeString`), so it doesn't depend on the input source
  — Korean types without switching the IME. Shortcuts (⌘V, ⏎) as key codes.
- **Aiming: never trust coordinates alone.** In the probe the target window's
  centre was covered by the person's terminal and the typed text went there.
  Before a click: bring the app forward through AX (`AXFrontmost`, `AXRaise`) and
  check with the **AX hit test** (`AXUIElementCopyElementAtPosition`) that the
  point reaches the intended app; the window list can't be used for this
  (BetterDisplay keeps a click-through overlay over the whole screen). Before
  typing: check `AXFocusedApplication`. Otherwise refuse and say why.
- **Apps: `open -a` / `open -b <bundle id>`** (what NSWorkspace does, no extra
  permission; `-F` so it doesn't reopen old windows). The app opens *behind* the
  frontmost one — a background process can't activate it — so raise it via AX.
  The app's pid from `NSRunningApplication`/`pgrep`.
- **Accessibility (AXUIElement): yes, as the first way to aim.** It gives roles,
  titles and frames of buttons and fields, `AXPress`, and setting `AXValue` —
  steadier than coordinates when a window moves or the scale changes, and it
  lets the tool *check* what happened (the probe reads TextEdit's text back).
  Coordinates stay as the fallback for whatever AX doesn't expose. Risk: Teams is
  a Chromium/WebView app — its web content shows up in the AX tree only after the
  app is told an assistive client is present (`AXManualAccessibility` /
  `AXEnhancedUserInterface` set on the app element). To verify on Teams in T-21.
- **Bindings: the `objc2` family** — `objc2-screen-capture-kit`,
  `objc2-core-graphics`, `objc2-foundation`, `objc2-application-services` (AX;
  the probe declares the few AX functions itself). Same family GPUI already
  pulls in; no `xcap`/`enigo`-style wrappers, whose capture paths are the
  deprecated ones and whose text input goes through key codes.

### TCC permissions

- **What needs what.** Screen: *Screen & System Audio Recording*. Clicks and
  keys: *Accessibility* (it also covers posting events; the probe checks
  `CGPreflightPostEventAccess` separately). AX tree: *Accessibility*. Opening
  apps: nothing.
- **Who it attaches to.** TCC checks the *responsible* process: for a launchd
  job, the job's own executable; every child (and grandchild) is charged to it
  unless spawned with responsibility disclaimed. So under the daemon, the grant
  belongs to the `equipe` binary — and every member's `claude`, every Bash
  command it runs, every MCP server inherits it. A member could then run
  `screencapture` from Bash, outside the one-at-a-time lease and outside any
  approval. **That's not acceptable.**
- **What survives an update.** TCC records the binary's code requirement. equipe
  ships ad-hoc, linker-signed (`flags=adhoc,linker-signed`, no team): the
  requirement is the **cdhash**, which changes with every build — each update
  silently drops the grant (System Settings may still show it on). A stable path
  (Homebrew `bin/equipe`, `~/.local/bin/equipe`) doesn't help; the signature does.
  Fix: sign with a **Developer ID** under a fixed identifier (requirement becomes
  "identifier X + Apple-issued cert of team T"), which holds across versions.
  Notarization isn't needed for TCC but comes with it for Gatekeeper.
  **Proven with the probe:** a self-signed certificate with a fixed identity
  already gives a requirement of the form "identifier + certificate leaf", and the
  grants held across two rebuilds. Developer ID is what makes that work on every
  user's Mac without a local certificate.
- **It has to be an app bundle.** Accessibility's "+" didn't take a bare
  executable at all; an `.app` with `CFBundleIdentifier`, `LSUIElement` and
  `NSScreenCaptureUsageDescription` appeared in both lists by itself after one
  `CGRequestScreenCaptureAccess` / `AXIsProcessTrustedWithOptions(prompt)` call.
  The LaunchAgent runs `<app>/Contents/MacOS/<exe>` directly — TCC still sees the
  bundle.
- **Updating the bundle in place hits App Management.** macOS blocked a process
  charged to `equipe` from modifying an app bundle (notification "blocked
  'equipe' from modifying apps"). The updater must either have App Management
  permission (one more toggle) or install the new bundle beside the old one and
  swap it (Homebrew's cask way); T-21 picks one.
- **Decision: a separate helper, its own binary and its own LaunchAgent.**
  `equipe-desktop` (identifier `im.equipe.desktop`), loaded as
  `im.equipe.desktop` in `gui/<uid>` with `LimitLoadToSessionType=Aqua`. Only it
  holds the grants; the daemon and the members never do. It is small and changes
  rarely, so even before Developer ID signing a grant lasts until the helper
  itself changes — and `equipe` can say so ("the desktop helper was updated:
  allow it again") by checking `CGPreflightScreenCaptureAccess` /
  `AXIsProcessTrusted` through the helper at start. One grant per helper
  version, made by the person with two toggles; `equipe desktop allow` opens the
  two Settings panes (`x-apple.systempreferences:` URLs) and waits.

### Session

- **Aqua, required.** Only a job in `gui/<uid>` sees the window server. The daemon
  itself doesn't need it once the helper exists — which is good, because today
  the daemon is easily in `Background` (started from SSH, or `daemon install`
  falling back to `user/<uid>` when nobody is at the console).
- **Login.** The helper only runs while the person is logged in at the console.
  After a reboot that needs automatic login — and macOS disables automatic login
  when **FileVault** is on, so a FileVault Mac mini waits at the unlock screen
  until a person logs in (Screen Sharing/Jump Desktop can do it remotely). The
  helper reports "no GUI session" instead of failing silently.
- **Lock screen.** While locked, capture shows the lock screen and input goes
  nowhere useful. Detectable: `CGSessionCopyCurrentDictionary` →
  `CGSSessionScreenIsLocked`. → T-22.
- **Sleep.** System sleep must be off (`pmset sleep 0`; here Amphetamine already
  keeps it). Display sleep is fine for capture on a virtual display; to verify on
  a physical one.
- **No monitor.** ScreenCaptureKit captures *displays*; with none attached there
  is nothing to capture (a headless Mac mini usually still has a 1× fallback
  framebuffer, but at a resolution that makes apps cramped). This Mac has one
  only because a virtual display is present. Recommend an HDMI dummy plug
  (≈₩10,000) or a virtual display (BetterDisplay / Jump Desktop) at 1920×1080;
  `equipe doctor` should report the display's size and scale.

### Images

- Capture at the model's size, not the screen's: long edge ≤ 1568 px (≈1.15 MP
  for Claude's vision; computer-use guidance prefers ~1280×800). ScreenCaptureKit
  does the scaling.
- Coordinates: the model answers in image pixels; the tool converts to screen
  points, `x_pt = display.x + x_img × (display.width_pt / image.width)`, same for
  y. CGEvent works in points, so Retina scale cancels out; the probe prints the
  factor. Multiple displays: main display only, at first.
- **Never written to disk** (ROADMAP Storage): the PNG/JPEG lives in memory and
  goes out as the MCP tool result. equipe's journal records one line ("screenshot
  1280×720"). Caveat: the engine keeps its own transcript, and Claude Code's
  JSONL stores tool-result images base64 — equipe can't prevent that, but should
  say so in `equipe storage`.

### Tool shape

- **MCP, like the browser:** members get `mcp__desktop__…` — `screenshot`,
  `elements` (AX tree of an app/window: role, title, frame, id), `click`
  (element id or x,y), `type`, `key`, `scroll`, `open_app`, `release`.
- **Served by the helper, reached through the daemon.** The helper listens on a
  Unix socket in `~/.equipe/`; the daemon gives each member an MCP endpoint
  (like `browser`) that forwards to it. The daemon holds the **lease**: one member
  has the screen; others get "the screen is busy (frontend is using it)" and
  can wait. The lease ends on `release`, end of turn, or idle timeout.
- Not in the daemon (it would need the grant → everyone inherits it), not a
  per-member child process (same problem, and no single lease).

## Send gate (T-23)

Sending anything to others in the person's name needs their yes. The rules are
code (`crates/core/src/desktop_gate.rs`), not the model's judgement; the runtime
puts the question (`Runtime::gate_desktop`, reached by `Handle::desktop_gate`).

**By app, not by keystroke.** A click at (x, y) or a Return doesn't say whether it
sends. So the gate looks at the app in front, as the helper reads it through AX
just before the input (`desktop_gate::Scene`: app name and bundle id, front
window title, the element a click lands on, the focused element and its text).

| In front | Input | Gate |
|---|---|---|
| any app | screenshot, `elements`, scroll, list/focus windows, `open_app` | free |
| a messaging app | `type_text` without a line break | free — a draft |
| a messaging app | `type_text` with `\n`/`\r`; `key` ending in Return/Enter (⌘↩, ⇧↩ too); ⌘⇧D (Mail) | asks |
| a messaging app | click on an AX element named Send/Share/Invite/Accept/Join/Call/Upload/…, or 보내기/전송/공유/초대/수락/통화/… | asks |
| a messaging app | click on an unnamed button, or where AX says nothing | asks |
| a messaging app | click on a named harmless element (a chat, a tab, the text field), space on a harmless focused element | free |
| a messaging app | drag | asks — it may drop a file into a conversation |
| any other app (TextEdit…) | anything | free |
| any app | typing into an `AXSecureTextField`; anything in Keychain Access, Passwords, 1Password, Bitwarden; a window or element named for payment, account deletion, passwords, security settings | **blocked**, in every mode |

Decisions:

- **Drafts go ahead.** Typing text without a line break only fills a field; the
  send (Return or the Send button) is the separate input that asks. The person
  sees the draft in the request (the focused field's AX value).
- **Asking costs a `why`.** An input that would ask, called without `why` (what's
  sent, to whom, in a line), is turned back at once without bothering the person;
  the desktop tools that make inputs take an optional `why`.
- **Never "always".** A send is asked every time, in every mode: `rule_for` has no
  rule for `Send`, so `a` isn't offered and an "always" from anywhere becomes once.
  No mode stops it, so the console doesn't suggest `/mode … full` under it.
- **The screenshot** goes to a file in the temp folder (mode 600) only while the
  person is asked — `o` opens it in Preview — and is deleted on the answer,
  including the 30-minute "nobody answered" no. It is the one image equipe writes,
  and only for this.
- **The window title doesn't block in messaging apps.** There it is a conversation's
  name ("결제팀"): matching it against the hard-block words would lock the person's
  team chat. Outside messaging apps it does (System Settings → Privacy & Security).
- **The list is the squad's** (`meta.messaging_apps`), defaulting to
  `desktop_gate::DEFAULT_APPS`. Entries match by shown name or bundle id; the
  defaults also match by bundle id, so "Microsoft Teams (work or school)" counts.
  `/send-apps`, `/send-apps add Notion`, `/send-apps remove Zoom`, `/send-apps reset`
  in the console. The daemon reads it at each input.

**For T-35 (the desktop tool):** before every `click`/`double_click`/`drag`/
`type_text`/`key`, read the `Scene` through AX (at the point for a click, the
focused element for typing), take a screenshot if it's cheap, and call
`Handle::desktop_gate(Check { … })`. `is_error` means don't make the input; return
`text` to the member. Inputs that need no AX (looks) may skip the call — `judge`
answers free for them anyway. A tool not named in `Act::of` is taken for a click.

## Remaining risks

1. **Clicking into the wrong window.** It happened in the proof: text meant for
   TextEdit went into the person's terminal. The AX hit-test and focus checks
   stop it in the probe; T-21 must keep them on every click and keystroke, and a
   keystroke stream must stop the moment focus moves (the person may click
   elsewhere mid-way).
2. **Unsigned releases** → each helper update asks the person again (proven the
   other way round: a fixed signature keeps the grants). Developer ID
   signing costs the Apple Developer Program ($99/yr) and a signing step in
   `release.yml`. Decision for the person.
3. **Teams' AX tree** may be thin until Chromium accessibility is switched on;
   then coordinates only, which are brittle.
4. **FileVault + reboot** stops the desktop until someone logs in.
5. **Engine transcripts keep screenshots** (Claude Code) even though equipe doesn't.
6. A member clicking in the person's session while they use the Mac collides with
   them: the lease should also yield when the person is active (HID idle time <
   N s), not only to other members.

## T-22 on this Mac: the person first

`cargo run -p equipe-core --example desktop_watch -- <seconds>` asks the gate
twice a second whether a member's input would go in, with this Mac's real
readings. It posts nothing. Run from a member's turn under the installed daemon
(`launchctl managername` → `Aqua`), 2026-10-14 00:4x–00:5x:

| Moment | Last input | Lock | Member input |
|---|---|---|---|
| The person working the Mac (Jump Desktop) | 0.0–9 s ago, reset at irregular moments | Open | held: "The person is using the Mac… at least 30 s more" |
| The person away | 32.9 s ago | Open | goes in |

So the idle clock sees the person's input over Jump Desktop, and nothing on this
Mac (Amphetamine included) resets it on its own: members aren't kept out
forever. Not yet tried, because no member can post input before T-35: a
member's own clicks not being taken for the person (unit-tested in
`desktop::presence`), and input stopping in the middle of a long call (left to
the helper, below).
## T-35: the tools on the real Mac

Built as decided above (`crates/desktop-control`, see ARCHITECTURE *Desktop
tools*), with two names changed: the helper is `im.equipe.desktop-helper`
(*equipe Desktop Helper*), since `equipe-desktop` is already the GPUI app. Run
on the person's Mac mini (macOS 26.2, Korean UI, display 1472×828 @1×),
2026-10-14, with a scratch home and a stand-in daemon, the member being a
headless `claude` given exactly the MCP config a member gets.

| Step | Result |
|---|---|
| Permissions missing, `screenshot` through member MCP → daemon → helper | ✅ refused: "isn't allowed to see the screen yet (missing: Screen Recording). Ask the person to run `equipe desktop setup`" |
| Helper bundle under `~/.equipe/desktop/…` | ❌ never listed in System Settings — Launch Services leaves out bundles in hidden folders. Moved to `~/Applications/equipe Desktop Helper.app` |
| Accessibility after that | ✅ listed; on after the person's toggle and password |
| Screen Recording | ⚠️ tccd logged `kTCCServiceScreenCapture … Auth Right: Allowed (System Set)` for the helper while `CGPreflightScreenCaptureAccess()` stayed **false**. The helper now asks ScreenCaptureKit (`SCShareableContent`) when the preflight says no |
| Rebuilt and re-signed helper, three times | ✅ every grant kept (same self-signed certificate) |
| Member: `open_app`, `key cmd+n`, `type_text` "equipe 데스크톱 테스트: 안녕하세요, 반갑습니다!" | ✅ typed into a new TextEdit document; nothing refused |
| Read back with `elements` | ✅ `AXTextArea = "Equipe 데스크톱 테스트: 안녕하세요, 반갑습니다!"` (TextEdit capitalised the first letter) |
| `screenshot` (1430×804 JPEG, 1 px = 1.03 pt) | ✅ the member described the window "무제 — 편집됨" and the same line |
| `app: "TextEdit"` on a Korean Mac | ❌ at first "no app called TextEdit": its name there is "텍스트 편집기". Apps now also match the last part of their bundle id |
| macOS's "bypass the private window picker" dialogs left up, focus moved to Chrome | ✅ `type_text` refused ("Keyboard focus is in no app, not 텍스트 편집기: nothing typed"); `click` into TextEdit refused: "At (735, 450) is UserNotificationCenter (AXButton "허용"), not 텍스트 편집기: nothing done" — it would have pressed the dialog's Allow |

Also seen: each capture attempt before the person answers adds another
"bypass the private window picker" dialog (they stack), and "Privacy & Security
blocked 'equipe' from modifying apps" when setup, run under the daemon, swapped
the bundle in `~/Applications` — the swap itself went through.

Not done here: Teams' AX tree (`AXManualAccessibility` is set, untested — no
Teams on this Mac), the screen lease (T-22), send approval (T-23).

## Notes for the follow-up tickets

- **T-21 (tools):** build `equipe-desktop` as its own binary + LaunchAgent
  (`im.equipe.desktop`, `LimitLoadToSessionType=Aqua`), not tools inside the
  daemon. AX first, coordinates as fallback. Images sized by ScreenCaptureKit,
  never on disk. Lease in the daemon, also yielding to the person (HID idle).
  `equipe desktop allow` / doctor checks for grants, session, display size.
  Verify Teams' AX tree with `AXManualAccessibility`.
  From the proof: the helper is an **`.app` bundle** with a fixed bundle id and a
  fixed signing identity; `request` on first start puts it in both lists. Every
  click is checked with the AX hit test, every keystroke with
  `AXFocusedApplication`, else the tool refuses. Apps are brought forward with
  `AXFrontmost`/`AXRaise`, not by `open`. Compare typed text as the app shows it
  (autocorrect/capitalisation change it). Plan the bundle update path around
  App Management. Ask the person to answer macOS's "bypass the private window
  picker" dialog during `equipe desktop allow`.
- **T-22 (lock screen):** detect with `CGSSessionScreenIsLocked`; the helper
  can't unlock (and mustn't hold the password). Tools return "the Mac is locked"
  and the member asks the person. Also cover "no GUI session" (logged out,
  FileVault after reboot).
- **T-22, done — for T-35 to use:** the daemon's gate is
  `Handle::desktop_ask(member, Use::Look|Input)` before each call goes to the
  helper, `desktop_done(member, start, input)` after it, and
  `desktop_release(member)` for the `release` tool; a refusal's text goes back
  to the model as the tool result. The gate decides *before* a call; a long
  call (`type` of a paragraph, a drag) must also stop *during* it when the person
  touches the Mac: the helper checks the last input's age between its own
  events and stops when the latest event is not one it just posted (same rule as
  `desktop::presence`).
- **T-23 (send approval):** with AX we can see *what* is about to be sent — the
  text in the compose field and the conversation title — so the approval shows
  the exact message and recipient, and the send (⏎ / Send button) is a separate
  tool call the daemon holds until the yes. Typing into the field is free; the
  send key in a messaging app is not.
- **New (proposed):** Developer ID signing + notarization in `release.yml`.
