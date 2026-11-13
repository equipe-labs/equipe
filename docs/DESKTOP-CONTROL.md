# Desktop control

> **Now:** members read apps through the accessibility tree and act on refs
> headless, with agent-desktop as the engine — see
> [Architecture now](#architecture-now-the-accessibility-tree-first-agent-desktop).
> The rest of this page is how the screenshot path came to be, and still holds
> for it.

> **Windows:** the same tools use Windows Graphics Capture for display/window
> images and UI Automation for the element tree, focus checks, Invoke actions,
> and window targeting. Text goes through Unicode `SendInput` (including Korean
> and surrogate pairs). The helper uses a local, remote-rejecting named pipe;
> calls still pass through the daemon's one-member screen lease, send gate, and
> `_scene` recheck before input is posted.

> **Linux:** the same tools, all through agent-desktop: the accessibility tools
> over AT-SPI, and the screenshot, pointer and keyboard tools over X11 (XTEST
> input, X11 capture), with the helper running inside the person's graphical
> session — see [Linux](#linux). `elements` isn't on Linux (`snapshot` with
> `bounds` is); Wayland has keys but no aimed pointer.

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

## Architecture now: the accessibility tree first (agent-desktop)

Members read and operate desktop apps mainly through the OS accessibility tree,
with [agent-desktop](https://github.com/vercel-labs/agent-desktop) 1.2.0
(Apache-2.0, Vercel Labs; THIRD_PARTY_NOTICES.md) as the engine. The
screenshot, coordinate and keyboard tools below stay as the fallback for apps
with a thin tree, canvases, shortcuts, and whenever a member truly needs pixels.

```
member ──mcp__desktop__snapshot / act / …──▶ daemon: may it? (desktop on) → its turn
   (Use::Read / Headless: no lease) → send gate on the scene → helper
helper = equipe.app itself (the one "equipe" Accessibility grant) ──spawns──▶ agent-desktop --session eq-<hash> …
   text on stdin · AGENT_DESKTOP_HOME=~/.equipe/desktop/agent-desktop/state (0700) · stderr dropped
```

### The integration choice: the CLI, run by the helper

Three ways in were weighed: the C library (`libagent_desktop_ffi`) in the
helper's process, the CLI as a child process, or `agent-desktop mcp`.

- **The CLI, chosen.** macOS charges a child's Accessibility use to its
  responsible process, so agent-desktop launched by the helper runs under the
  helper's grant — the one switch the person already turns on (for "equipe",
  see [One permission entry](#one-permission-entry-equipeapp-is-the-helper)). The person
  never grants agent-desktop itself (the release binaries are ad-hoc signed, so
  a grant to them would not survive an update anyway). The CLI has everything
  the members' loop needs: `--skeleton` and `--root` drill-down, `--window-id`,
  surfaces, `find`, `wait`, notifications, the clipboard, and `--stdin` for
  text. A crash or a hang costs one call (killed after its deadline), never the
  helper. One process per call is tens of milliseconds next to the AX walk
  itself.
- **Not the C library.** In process it would also keep the single grant, but
  at 1.2.0 its `ad_snapshot` targets only the focused window and exposes no
  skeleton, no `--root` drill-down and no window targeting — exactly what makes
  big apps cheap to read. Binding it means hand-written layouts for a few dozen
  size-pinned C structs in a crate equipe can't test against the real library
  here, a `panic = "unwind"` cdylib in the process that holds the person's
  grants, and a crash taking the helper down.
- **Not MCP.** `agent-desktop mcp` applies `--session` to the whole server, so
  per-member namespaces would mean a long-lived server per member, and its
  envelopes would still need equipe's gate, compaction and stdin handling in
  front of them. Per-call CLI is simpler and testable with a stand-in binary.

### Pinned, verified, private

- One release: `engine::pin::VERSION` = 1.2.0. equipe downloads the upstream archive for the machine (macOS arm64/x86_64, Windows
  x64/arm64, Linux glibc x86_64/aarch64 — whose archive also carries
  `agent-desktop-linux-clipboard-owner`, which agent-desktop runs to keep the X11
  clipboard after setting it), checks its SHA-256 against the value pinned in code (from the
  release's `checksums.txt`; each archive's Sigstore attestation was checked with
  `gh attestation verify` when pinning), unpacks only the pinned files with the
  system `tar`, checks each file's SHA-256, and installs them with a copy of the
  Apache-2.0 license in `~/.equipe/desktop/agent-desktop/1.2.0/` — inside the
  helper's folder, which members' sandboxed commands can neither read nor write.
  The helper hashes the executable again before its first run and whenever the
  file changes; a binary that isn't the pinned one never runs.
- **Upgrading:** change `VERSION` and every hash in `pin.rs` from the new
  release's `checksums.txt` and its unpacked files (`shasum -a 256`), run
  `gh attestation verify <archive> --repo vercel-labs/agent-desktop`, re-read
  the release's JSON-output changes against `engine/compact.rs`, and run the
  engine tests.
- **Private content stays on the Mac.** Snapshots, clipboard text and
  notifications go back to the member as the tool's answer and nowhere else:
  the helper logs only the tool's name, discards agent-desktop's stderr, never
  turns on its tracing (`--session` without a manifest records nothing), and
  empties its state root (each member's refmaps) whenever the helper starts.
  Nothing reaches telemetry. Text a member types (`act` type/set_value,
  `clipboard` set) goes in on stdin — never in process arguments, which any
  local process can read — and a password field's value is never shown.

### Tools

| Tool | What | Screen |
|---|---|---|
| `snapshot` | the app's tree, interactive elements only by default, as compact text (`e12 button "Send"`); `skeleton` then `root` to drill; `surface` for menus, sheets, alerts | none (`Use::Read`) |
| `find` | elements by role / name / text / value | none |
| `act` | on a ref: click, type, set_value, clear, focus, select, check, uncheck, toggle, expand, collapse, scroll, scroll_to — headless | none (`Use::Headless`) |
| `act` with `headed`, double_click, right_click | physical pointer/keys via agent-desktop (it focuses the window and hit-tests); a context menu opens on screen and holds the pointer | lease + person first |
| `apps` | list; launch (background); launch with `activate`, close | none / none / lease |
| `windows` | list; focus, minimize, maximize, restore, resize, move | none / lease |
| `wait` | an element's state, a window, text, a menu, an event (`window-opened`…), a pause | none |
| `notifications` | list (when Notification Center is open); act on a button, dismiss — by the listing's index, the listing's app as agent-desktop's fingerprint | none / lease |
| `clipboard` | get text; set (stdin), clear | none / lease (the person may be about to paste) |

Refs are printed short (`e12`, meaning the member's last snapshot) and passed
back as printed or qualified (`s8f3k2p9:e12`). A ref no snapshot of this
member's showed in this helper run is refused before anything runs; each
member has its own agent-desktop session (`eq-` + a hash, so no names on disk).
Errors come back as one line that says whether a retry is safe
(`disposition.retry`), and what to do (`STALE_REF` → snapshot again). The
member skill `desktop-apps` teaches the loop: snapshot → act on a ref →
snapshot the same region to verify; retry only when safe.

### Freedom and concurrency

- `desktop::Use` gained `Read` and `Headless`. Neither takes the lease or
  queues, and neither waits for the person's hands: a headless ref action
  moves no pointer and takes no focus, so several members work different apps
  at once, beside the person. `Headless` is still refused while the Mac is
  locked or nobody is logged in (`Screen::Locked`/`NoSession`); `/desktop stop`
  refuses both. Physical input — the screenshot tools, `act` with `headed`,
  double-click and right-click (a context menu holds the pointer), focusing or moving windows, launching with `activate`,
  notification buttons, writing the clipboard — keeps the one-member lease and
  waits for the person to leave the Mac alone (`PERSON_QUIET`).
- In the helper, each accessibility call runs on a thread of its own; the
  screenshot tools stay one at a time. agent-desktop serializes its own
  mutations across processes (its interaction lock), and re-identifies a ref
  before acting, so two members can't act on a stale picture of each other's work.
- A member's headless work doesn't count as the person's input
  (`desktop_done(.., physical = false)`).

### The person's gates, on ref actions

The send gate (`desktop_gate`, unchanged rules) judges ref actions by code:
`Act::of("act", …)` makes click/select/check/toggle a `Click`, type/set_value a
`Type` (clear an empty one), focus/expand/collapse/scroll a `Look`; a
notification's button press is a `Click`. The helper's `_scene` for a ref comes
from the member's snapshot: the snapshot's app (and its bundle id, from
`list-apps`, so a localised Mail still counts as Mail), its window title, the
ref's role mapped to the AX role the gate reads (`button` → `AXButton`, a
`secure` text field → `AXSecureTextField`) and name, and the draft — the
focused or last non-empty text field of the same snapshot. So:

- pressing Send / Join / Share by its ref in a messaging app asks the person,
  every time, with the draft and a screenshot (when Screen Recording is on);
  writing the draft goes ahead; a line break in typed text asks;
- payment, account deletion, passwords and security settings, password
  managers, password fields and equipe's own windows take no input;
- after a yes the daemon passes `_asked`, and the helper re-reads the window
  title (the conversation) and the draft live before acting — another
  conversation or another draft than the person read, and nothing is sent.

### One permission entry: equipe.app is the helper

System Settings used to list two rows, "equipe" (the app, with its icon) and
"equipe Desktop Helper" (a blank icon) — the second a signed copy of `equipe`
in `~/Applications` that held the permission. Now:

- **With equipe.app installed** (any version whose Info.plist has
  `EquipeDesktopServe`), the helper's LaunchAgent (`im.equipe.desktop-helper`)
  runs the app's own executable: `equipe.app/Contents/MacOS/equipe-desktop
  desktop serve --home ~/.equipe`. That mode opens no window; it serves the
  helper socket exactly as before. macOS judges a permission by the code that
  runs, so the app's single "equipe" row covers the app, the helper and
  agent-desktop under it. Nothing is copied or signed on the Mac: the app's
  Developer ID signature keeps the grant across Sparkle updates, and the
  LaunchAgent's path never changes (no "background item added" again). After
  an update the daemon restarts the helper once so it runs the new build.
- **Without the app** (the command line alone) the helper is still a bundle in
  `~/Applications`, now named "equipe" with equipe's icon (folder name kept,
  so an existing grant stays). It is rebuilt only when the helper's code
  changed (`install::REVISION`, a hash of `crates/desktop-control/src`), not on
  every update; a new build is staged beside the old one and swapped in by
  renaming, and the old one is moved to `~/.equipe/desktop/retired/` — never
  written into or deleted, which is what macOS's App Management blocked
  ("macOS blocked equipe from modifying apps").
- **Moving over.** A Mac that had the old bundle and gets equipe.app: the
  LaunchAgent is pointed at the app once. While the app isn't allowed yet and
  the old bundle is, the app shows macOS's Accessibility prompt for "equipe"
  once and hands over to the old bundle (`exec`, so macOS judges the old
  bundle's grant) — members keep working. The daemon starts the app again on
  its own start, when the person presses Allow…/Check, and every 6 hours, to
  see whether "equipe" is allowed by then. Once it is, the old bundle is moved
  aside and the person is told once — a card in the app, a note in `equipe
  desktop status` — that the "equipe Desktop Helper" row can be removed (−).
- `equipe desktop status` says which process holds the permission ("runs as
  equipe itself", or the bundle, or "the older helper keeps working
  meanwhile").

### Permissions UX

- **One grant matters: Accessibility for "equipe".** It covers
  the tree, ref actions and input, and agent-desktop under it. Screen
  Recording is optional now — only screenshots and the send-approval picture.
- **Every member works the desktop by default** once that switch is on — no
  `equipe desktop on <member>` step. The person opts out per member
  (`equipe desktop off <member>`) or per squad (`equipe desktop off
  --everyone`; `on --everyone` undoes it); a member turned on by name works it
  either way (and the switch is then asked for). Every gate below stays:
  sending in the person's name asks them, payment/password/security screens
  and password managers take no input, text goes in on stdin, and nothing of
  the screen is logged.
- **Prompt budget.** Every macOS prompt equipe can cause goes through one place
  (`equipe_desktop_control::prompts`): at most one of a kind is up at a time
  in a process, the answer is kept in `~/.equipe/prompts.json`, and after a
  yes equipe never asks again for that identity unless macOS takes the grant
  back; after a no, background work waits a day. The Accessibility prompt is
  shown once per identity (the app, or the bundle).
- The daemon sets all of it up by itself — see [Automatic setup](#automatic-setup)
  below. `equipe desktop setup` does the same right away and walks the person
  through the grant; `equipe desktop status` lists every check (GUI session,
  daemon, engine installed and verified, helper, Accessibility, input, the
  engine working under the helper's grant, Screen Recording, display, lock) — in
  the person's language (8 locales).
- **Settings › Squad › Desktop apps** in the app shows the same
  (`Command::DesktopCheck` → `Event::DesktopReady`), with **Allow…**: the
  daemon asks the helper to request its permissions (macOS's prompts) and opens
  System Settings at the missing pane — on the daemon's machine, which is where
  the switch must be flipped, even when the app is connected over SSH.

### Cost: tree versus screenshot

`engine::tests::snapshot_costs_a_fraction_of_a_screenshot`
(`cargo test -p equipe-desktop-control snapshot_costs -- --nocapture`), on a
hand-written chat window in agent-desktop's documented shape (a Slack-like
channel: 20 conversations, 8 messages with their buttons, a composer with a
draft; 75 refs):

| What the member reads | Bytes | ≈ Tokens |
|---|---|---|
| agent-desktop's raw JSON, `-i --compact` | 10,270 | 2,568 |
| equipe's compact tree, whole window (92 lines) | 2,749 | 688 |
| equipe's compact tree, `skeleton` | 271 | 68 |
| one screenshot, 1330×865 (a 1470×956 pt display at 2×) | — | ≈1,534 (w×h/750), plus the base64 JPEG the engine keeps in its transcript |

Text at ~4 bytes a token. The usual loop — skeleton (68), drill into the
composer (~100), act, re-read the composer (~100) — costs a few hundred tokens
where screenshots cost ~1,500 for every look, and the answer says exactly what
changed instead of leaving the model to find it in pixels. The raw JSON alone
would cost more than a screenshot: compaction (short refs, no
`available_actions`, empty wrappers folded, values cut) is what makes the tree cheap.

### Automatic setup

The person's only step is the one switch macOS keeps for them. Everything else
is the daemon's (`equipe-core`, `desktop::auto`; every machine-touching call
sits behind its `Ops` trait, so the whole of it is tested with fakes that touch
no permission, no screen and no network).

| Step | Who | When |
|---|---|---|
| Engine: download, verify the pinned SHA-256s, install | daemon, in the background, off the main loop (15 s after it starts) | the engine is missing, doesn't verify, or `VERSION` changed (`equipe update` restarts the daemon) — for every user, from the first daemon start |
| Helper: point its LaunchAgent at equipe.app (or, without the app, install and sign the bundle), keep it current | daemon | once the desktop is wanted (the helper was set up once, a member was turned on by name, or a desktop tool was called) and someone is logged in at the Mac |
| macOS's Accessibility prompt for "equipe" | daemon, **once** per identity (`_request_accessibility`, through the prompt budget) | engine ready, helper answering, grant missing, desktop wanted |
| Moving from the old bundle to equipe.app, and the note that its row can go | daemon + the app's helper mode | equipe.app installed and allowed |
| The switch | **the person** | the card below, or the first-run step |
| Watching for the switch, the status check, "Desktop apps ready" | daemon: a light poll (4 s for 10 minutes, then 30 s) while the card is up | the grant appears |

equipe never tries to get around TCC: no `tccutil`, no edits to the permission
database, no clicks on System Settings. The card's button opens System Settings
at Privacy & Security › Accessibility (the same pane "Allow…" opens, on the
daemon's machine even over SSH) with the helper already listed; the system's
own prompt was shown once before it.

- **A failure** (offline, a checksum that doesn't match, the helper can't be
  installed) is shown once as a card in plain words with **Try again**, and
  tried again quietly with a growing wait (2 min, 10 min, 1 h, 6 h). It never
  blocks the daemon, and the screenshot tools keep working. It is only shown
  when the desktop is wanted.
- **No nagging.** "Not now" puts the card away (`~/.equipe/desktop/setup.json`).
  It comes back only when a member actually needs the desktop and can't have it
  (a tool call refused because the switch is off), and then at most once in six
  hours — except after skipping in the first run, which is undone by the first
  member who needs the desktop.
- **Members** are told in one short line what is missing and whether to retry:
  "equipe is still setting up the engine, call again in a minute", or "needs one
  switch only the person can flip — don't retry, do something else". Screenshots
  need neither the engine nor Accessibility and are never held back by it.
- **First run.** The desktop app's first-run screen (and the terminal's
  onboarding, on a Mac) has one optional step, "Desktop apps": it explains in
  two sentences, then sets up on a thread of its own (no daemon is in the loop
  yet), shows macOS's prompt, and offers **Open System Settings** until the
  switch is on. **Skip for now** is a full answer.
- **Managed Macs (MDM).** Accessibility can be pre-approved by a configuration
  profile on a supervised or MDM-managed Mac, so nobody flips anything: push a
  Privacy Preferences Policy Control (PPPC) payload (`com.apple.TCC.configuration-profile-policy`)
  with an `Accessibility` entry for equipe.app, `Authorization` = `Allow`, with
  `Identifier` = `im.equipe.desktop`, `IdentifierType` = `bundleID` and the
  `CodeRequirement` that `codesign -d -r- /Applications/equipe.app` prints
  (`equipe desktop setup` prints it as well) — the release's Developer ID, the
  same on every Mac. Without the app, the entry is for the bundle
  (`im.equipe.desktop-helper`, `codesign -d -r- "~/Applications/equipe
  Desktop Helper.app"`), whose requirement depends on how it is signed: a
  Developer ID (`equipe desktop setup --sign "<identity>"`), since a
  self-signed certificate is different on every Mac. When the profile
  is in place the daemon sees the grant on its first look and says "Desktop
  apps ready" without a card. Only Accessibility can be pre-approved this way;
  Screen Recording can only be allowed by the person (a profile can merely let
  standard users approve it). equipe ships no profile of its own.
- **Windows** needs no grant: UI Automation works in the person's session. The
  daemon downloads the engine automatically and starts the helper
  (`equipe desktop serve`, detached) when a member first needs the desktop; a
  helper left over from an older equipe isn't replaced automatically yet.
- **Linux** needs no grant either, but a session: see [Linux](#linux). The
  daemon downloads the engine and, once the person has run `equipe desktop
  setup` (or a member needs the desktop), starts the helper detached inside the
  person's graphical session; when there is no session to reach it says what
  is missing instead.
- Opt out of all of it with `EQUIPE_NO_DESKTOP_AUTO=1` in the daemon's environment.

### What needs the person on a real Mac

Not run against a live desktop while building this (by rule): everything above
is tested with fakes and a stand-in agent-desktop. On a real Mac, the person:

1. does nothing for the engine and the helper;
2. switches on **Accessibility** for "equipe" (password or Touch ID) when the
   card, the first-run step or macOS's prompt asks — the one switch;
3. optionally switches on **Screen Recording** for it (screenshots), from
   Settings › Squad › Desktop apps › Allow…;
4. can check `equipe desktop status`: "The engine works under the helper's
   Accessibility permission" must be ✓. If macOS lists `agent-desktop`
   separately and the line stays ✗, the responsible-process attribution didn't
   hold on that macOS version: grant the listed binary too and report it.

To verify end to end: a member with the desktop on snapshots TextEdit, types
into its text area by ref, re-reads it; then in a messaging app, a ref click on
Send must ask.

### Working while the screen is locked

An unattended Mac (a Mac mini in a cupboard) is locked most of the time. The
person may turn on **Work while the screen is locked** (`equipe desktop locked
on`, or the switch in Settings › Desktop apps; off unless turned on; the
person's alone — members can't change it):

- **Allowed while locked:** reading (`snapshot`, `find`, `wait`, lists — these
  always were) and **headless ref actions** (`act` without `headed`: press,
  set a value, select, toggle, scroll, launch in the background). They go to
  the app through the accessibility API (an `AXUIElement` call answered by the
  app's own process), not through the window server's input path, and the
  apps keep running in the locked session.
- **Still waits for an unlocked screen:** everything physical — `headed`
  actions, double/right clicks, focusing or moving windows, launching with
  `activate`, writing the clipboard, notification buttons, and the
  screenshot-and-click tools. macOS routes synthesised pointer and keyboard
  events to the lock screen while it is up (they would type into the password
  field, if anything), and screen capture shows the lock screen. Members are
  told so in one line (`Refusal::LockedPhysical`) and pointed at `act` on a ref.
- **Not verified on a live Mac** (by rule, nothing here touched a real
  screen): some apps stop updating their tree, or ignore actions, while the
  session is locked (App Nap, a window server that stopped drawing them).
  When a headless action fails while the screen is locked, the member is told
  that this may be why, and to read the app again or wait for the person.
- **Logged out stays refused**, and so does FileVault before the first login:
  there is no session, and no app, to act in. `/desktop stop` refuses
  everything as before.

### Linux

Linux has no Accessibility switch to flip: whether agent-desktop works depends
on whether the person's desktop session can be reached and its AT-SPI bus
answers (upstream: <https://agent-browser.dev/desktop/platforms/linux>). So the
Linux helper (`crates/desktop-control/src/linux.rs`) is about the session:

- **What it checks** (`linux::probe`), each with a line the person can act on:
  1. a display — `WAYLAND_DISPLAY` with its socket in `$XDG_RUNTIME_DIR`, or
     `DISPLAY` with its X socket in `/tmp/.X11-unix` (a TCP display is taken on
     trust); a variable naming a session that is gone is told apart from none;
  2. the D-Bus session bus — `DBUS_SESSION_BUS_ADDRESS`, or systemd's per-user
     bus at `$XDG_RUNTIME_DIR/bus`;
  3. AT-SPI — `org.a11y.Bus.GetAddress` on the session bus, asked with
     whichever client is installed (`gdbus`, `busctl`, `dbus-send`);
  4. (advisory) `org.a11y.Status.IsEnabled`: whether toolkits are told to
     expose their trees (`gsettings set org.gnome.desktop.interface
     toolkit-accessibility true` on GNOME; `QT_ACCESSIBILITY=1` for Qt apps).
  The final word is agent-desktop's own: the helper runs `agent-desktop
  permissions` and reports its `accessibility` state, which on Linux is
  `granted` exactly when the bus answers.
- **Where the helper runs.** The daemon is a systemd user service, whose
  environment may lack the session's variables. `linux::this_session` takes
  the process's own and fills the missing ones from `systemctl --user
  show-environment` (desktops export `DISPLAY`, `WAYLAND_DISPLAY`,
  `XAUTHORITY` there at login). The helper (`equipe desktop serve`) is started
  detached in its own process group with exactly those variables, logs to
  `~/.equipe/desktop/helper.log`, and keeps its pid in
  `~/.equipe/desktop/helper.pid` so `equipe desktop setup` can restart one left
  over from an older session and `equipe desktop remove` can stop it. It
  listens on the same `0600` Unix socket with the same token as on a Mac.
- **`equipe desktop status` / `setup`**, from a terminal inside the session,
  print the checks above, the engine, whether the helper answers and whether
  agent-desktop reads apps through AT-SPI from inside the helper. `setup`
  installs the engine, starts the helper (or restarts a stale one) and exits 1
  with what is still missing. From a bare SSH shell or a TTY both say "No
  graphical session…" and "No D-Bus session bus…", not a blanket refusal.
- **Set up once, everyone's by default.** `equipe desktop setup` leaves a mark
  (`~/.equipe/desktop/linux-setup`) the daemon reads as "the desktop is
  wanted", as the helper's LaunchAgent is on macOS; the daemon starting the
  helper itself leaves it too, and `equipe desktop remove` takes it away. With
  the mark, the daemon keeps the helper running, sees AT-SPI answer, and every
  member has `mcp__desktop__…` from its next turn without `equipe desktop on`
  (still off for whoever the person turned it off for). Before this, nothing
  made the desktop wanted on Linux until someone ran `equipe desktop on`, and
  members on a fresh machine never got the tools.
- **The daemon's own setup** (`desktop::auto`): no display or no bus → no
  helper is started and `DesktopReadiness.missing` carries the lines; a member
  who needs the desktop is told what is missing (not "the helper is
  starting"), and that makes it a `Failed { reason: Session }` card the person
  can retry. AT-SPI not answering → the same, from the helper's `missing`.
  There is never an Accessibility "Grant" card on Linux, and nothing is
  prompted for.
- **The rules don't change.** Members' calls go through the same daemon
  checks: `/desktop stop`, the lease for anything physical, the person-first
  wait, the send gate and the `_scene` recheck. `ThisMac` on Linux reads the
  session for the lease: no display or bus is `NoSession`; the screensaver's
  `GetActive` (GNOME's, then freedesktop's) is the lock; the person's idle time
  comes from GNOME's `org.gnome.Mutter.IdleMonitor`, else the X server's own
  count (MIT-SCREEN-SAVER, asked directly with `x11rb` on the session's
  display with its Xauthority cookie — Xfce, openbox and other non-GNOME X11
  desktops), else `xprintidle` — and when none answers, pointer and keyboard
  input is refused, since not knowing whether the person is there is not leave
  to go ahead (`equipe desktop status` shows whether idle time is readable).
  Headless ref actions and reads need none of it.
- **Screenshots, pointer and keyboard** (`crates/desktop-control/src/linux_desktop.rs`).
  The helper's `Desktop` for the controller is agent-desktop again, one short
  child process per step, with snapshots kept apart from the members' own
  (`~/.equipe/desktop/agent-desktop/pixels`):
  - `screenshot`: `screenshot --screen 0` (or `--window-id` for one window),
    resized and sent as JPEG like on a Mac. Under a compositing window manager
    (xfwm4 with its compositor on, GNOME on X11) the root window holds no
    picture and agent-desktop answers `PLATFORM_NOT_SUPPORTED`: the helper then
    puts the screen together from each window's own picture — a window covering
    the display (the desktop) first, the focused window last — so coordinates
    still mean the whole screen; what no window owns is black. A window
    screenshot works either way.
  - `click` / `double_click` / `drag` / `scroll`: `--headed mouse-click`,
    `drag --from-xy --to-xy`, `mouse-wheel --x --y --dy --dx` (real XTEST
    events). Before any of them the controller's own check: the app is brought
    forward (`focus-window`) and the point must be in its window — "what is at
    a point" is the focused window when it covers it, else the first listed
    window that does (X11 has no Accessibility hit test; an overlay that isn't
    a window isn't seen).
  - `type_text`: real keys into the element whose AT-SPI state is `focused`
    in the focused window (`type --headed <ref> --stdin`, the text never in
    the arguments), in runs between the return and tab keys a newline and a
    tab press, focus checked before each run. Keys agent-desktop couldn't send
    at all (it can't confirm the window active, as under openbox) go into the
    same focused field through its text interface instead. Where nothing
    focused takes text (a terminal), key by key with `press` when every
    character is a key. The
    send gate sees that focused field, so a password field gets nothing, as on
    a Mac.
  - `key`: `--headed press`, `cmd` read as Ctrl (as on Windows). agent-desktop
    refuses combos that switch the session or close a window (`alt+f4`,
    `ctrl+alt+…`) and the member is told.
  - `list_windows` / `focus_window` / `open_app`: `list-windows`,
    `focus-window`, `launch`. Window ids are agent-desktop's (34–70
    characters), numbered `w1`, `w2`… for members by the helper. Apps with a
    window on screen come first, so a member's headless browser (no window)
    never stands in for the person's of the same name.
  - Every input failure (a refused combo, Wayland's unaimable pointer) comes
    back to the member instead of "Clicked".
  - A headed `act` click that agent-desktop times out on before sending
    anything (`TIMEOUT`, `not_delivered`: Chromium's links never read as
    actionable over AT-SPI) goes in with the pointer at the element's centre
    (`get --property bounds`, then `mouse-click`), and says so; one that may
    have landed is only told.
- **Not on Linux yet:** `elements` (refused, pointing at `snapshot` with
  `bounds`, `find` and `act`); the helper isn't replaced automatically after an
  equipe update (as on Windows: `equipe desktop setup` restarts it); a helper
  isn't started at login by itself (the daemon starts it once set up). From
  upstream: Wayland has no aimed pointer, no window screenshot and no window
  control (keys go through Mutter's RemoteDesktop; the first screenshot asks
  the person through the portal), headless `scroll` is refused,
  notifications are GNOME Shell only, and Ubuntu 22.04's AT-SPI bridge is too
  old (24.04 or later, or an `at-spi2-core` with upstream's fix).
- **Chromium and Chrome** expose their web content only with
  `--force-renderer-accessibility`. With the system's `org.a11y.Status.IsEnabled`
  on, other Chromium processes (a member's headless Playwright browser) join
  the bus as a second `chrome`: the screenshot tools tell them apart by window
  (above); for `snapshot --app chrome`, name the window (`window` from
  `windows`).
- **Packages** on a minimal machine (what the live run used):
  `at-spi2-core`, `libatk-adaptor` (GTK's bridge), `dbus-x11` or another
  session bus, a window manager, and for X11 headless work `xvfb`; Wayland
  clipboard needs `wl-clipboard`.
- **Run live**, with the scripts and output in
  [`research/desktop-control/linux`](./research/desktop-control/linux):
  - *A real Xfce desktop* (Ubuntu 24.04 x86_64 VM: Xvnc, xfwm4, Mousepad and
    Chrome; `vm-xfce.txt`): from a squad with no `desktop on` and no remembered
    readiness, `equipe desktop setup` inside the session reported ready (idle
    time from the X server: the VM has neither Mutter nor `xprintidle`); the
    daemon, a systemd user unit started without `DISPLAY`, found the session
    through the user manager, read the setup mark and called the desktop ready
    by itself; then one real Claude Code member on an action ticket took a
    whole-screen screenshot, clicked into Mousepad, pressed `ctrl+end return`
    and typed a line with `type_text` (real keys), took a second screenshot to
    check, and handed in after 65 s. The editor's text was read back
    independently. With xfwm4's compositor on, agent-desktop's whole-screen
    capture is `PLATFORM_NOT_SUPPORTED` and the helper's composed picture came
    back in 1.5 s.
  - *A container* (Ubuntu 24.04 aarch64: Xvfb + openbox + AT-SPI + GTK
    `zenity --entry`; `run.sh`, `live.txt`): `status`/`setup` from a bare
    shell and without AT-SPI say what is missing; in the full session the
    accessibility path (snapshot, `_scene`, `act type` verified, `act click`)
    and the screenshot/pointer/keyboard path (screenshots, `list_windows`, a
    click outside the app refused before any input, `type_text`, a blocked
    `alt+f4` told, `return` closing the dialog with the typed text) work.
  - *Not run live:* Wayland (keys through Mutter, the portal's consent), GNOME
    Shell, the systemd environment fallback with a session that exports
    nothing, lock detection, the Chromium headed-click fallback (unit-tested
    only), and a person using the machine while a member waits for the screen
    (the person-first wait is platform-independent and unit-tested).

### Limits and follow-ups

- **Sandboxes (separate desktops): follow-up.** agent-desktop's `sandbox`
  command (a macOS VM provider) landed upstream after 1.2.0 and isn't in a
  release yet; its guest needs a person to finish setup and its own
  Accessibility grant. When a release ships it: pin it, expose `sandbox`
  create/start/status/stop to the person (not members), and route a member's
  calls through `sandbox exec <name> --` so risky or long app work runs off the
  person's desktop.
- Headless delivery depends on the app: Electron/Chromium apps may need a long
  `timeout_ms` to expose their web tree, some controls only take `headed` input,
  and macOS notifications list headless only while Notification Center is open.
- On Windows the helper runs accessibility calls inline (one at a time); the
  daemon still lets them run without the lease.
- The helper keeps up to 12 snapshots' refs per member in memory for the gate,
  and forgets them when it restarts; members then snapshot again.

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
| **Click at a point, type text incl. Korean, read it back** | ✅ pointer moved to (503, 299), keyboard focus on TextEdit (`AXFocusedApplication`), typed `hello from equipe probe` and a Korean greeting (*annyeonghaseyo*, in Hangul) as Unicode events; AX read back the same text, `Hello …` (TextEdit auto-capitalised the first letter). Seen on the capture too |
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
| a messaging app | click on an AX element named Send/Share/Invite/Accept/Join/Call/Upload/…, or the same words in any of equipe's languages (`match.outward_words`) | asks |
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
  name ("Payments team"): matching it against the hard-block words would lock the person's
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
| Member: `open_app`, `key cmd+n`, `type_text` in Korean, "equipe desktop test: hello, nice to meet you!" written in Hangul | ✅ typed into a new TextEdit document; nothing refused |
| Read back with `elements` | ✅ `AXTextArea` held the same line, `"Equipe …"` (TextEdit capitalised the first letter; the Korean typed as written) |
| `screenshot` (1430×804 JPEG, 1 px = 1.03 pt) | ✅ the member described the window — its title in Korean, "Untitled — Edited" — and the same line |
| `app: "TextEdit"` on a Korean Mac | ❌ at first "no app called TextEdit": its name there is the Korean for "text editor". Apps now also match the last part of their bundle id |
| macOS's "bypass the private window picker" dialogs left up (in Korean on this Mac), focus moved to Chrome | ✅ `type_text` refused ("Keyboard focus is in no app, not <TextEdit's Korean name>: nothing typed"); `click` into TextEdit refused: "At (735, 450) is UserNotificationCenter (AXButton "Allow", in Korean), not <TextEdit's Korean name>: nothing done" — it would have pressed the dialog's Allow |

Also seen: each capture attempt before the person answers adds another
"bypass the private window picker" dialog (they stack), and "Privacy & Security
blocked 'equipe' from modifying apps" when setup, run under the daemon, swapped
the bundle in `~/Applications` — the swap itself went through.

Not done here: Teams' AX tree (`AXManualAccessibility` is set, untested — no
Teams on this Mac), the screen lease (T-22), send approval (T-23) — both wired
in since (T-128, above).

## One desktop call, end to end (T-128)

`Handle::desktop` (the gateway's `desktop` request) runs every member's call
through both gates before the helper touches the Mac:

1. **May it** — the desktop is on for this member (`desktop_allowed`).
2. **Its turn** — `desktop_ask(member, Use::of(tool))`: screenshot, `elements`
   and `list_windows` are `Look`; everything else is `Input`, refused while the
   person is at the Mac, the screen is locked, or after `/desktop stop` (which
   refuses looking too). Another member's call waits in line.
3. **The send gate**, for an input (`Act` not `Look`): the helper's `_scene`
   request reads what it would act on — app, front window, the element a
   click lands on (a hit inside a button is the button), the focused element
   and its draft — without acting. `desktop_gate` judges it; when it asks, the
   helper's `_capture` (whole screen, members' coordinates untouched) is the
   preview. After a yes the call asks for its turn again as `Use::Approved`:
   the person's own hands on the Mac (answering) don't hold that one send back,
   another member holding the screen or `/desktop stop` still do.
4. **The helper acts** with `_expect: <scene>` in the arguments, and refuses if
   the screen isn't that scene any more — another app, conversation or field,
   or (after a yes) another draft — so a yes never lands somewhere else, and a
   field that became a password field gets nothing.
5. `desktop_done` records when it ran, so the members' own input isn't taken
   for the person's.

Tools that may send take `why` (what, and to whom): without it a send in a
messaging app goes back to the member unasked.

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
