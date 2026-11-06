# Mobile QA: members test the app on a phone

When one of a squad's projects is an iOS or Android app, its **QA** and **mobile** members can test
it the way a person does: start the app on a simulator or emulator, see the screen, tap, type,
swipe and scroll, read what is on the screen, **record a whole flow as a video and check it frame
by frame**, and hand the person a report — a verdict per step with the frame that shows it, and
the recording to play.

Nothing changes for any other member, or for a squad without a mobile app: their charters, skills
and tools stay byte for byte what they were (`crates/core/tests/members_without_phone.rs`).

## What a member gets

The `phone` MCP server (`mcp__phone__…`), served by `equipe mcp --server phone` and run by the
daemon (`crates/core/src/phone/`). How to use each tool is in its own description; the loop to
follow is the `mobile-qa` skill (`crates/core/skills/mobile-qa/SKILL.md`).

| Tool | Does |
|---|---|
| `start` | makes (or reuses) the ticket's simulator or emulator for this member, boots it without a window, installs a built `.app`/`.apk` and launches it; answers with the device id to build for |
| `look` | a screenshot in points and the screen's accessibility elements, one a line: `Button "Pay" id=pay @ 20,700 362×50` |
| `act` | `tap` (by label, `id:<id>` or a point), `type`, `scroll`, `swipe`, `key` (enter, delete, tab, escape, back, home), `wait` for an element to appear or go, `launch`, `open_url`; answers with the screen's elements after it |
| `record` | `start`/`stop` a video of the screen; each `act` while recording is marked on its timeline |
| `frames` | frames of a recording: before and after each marked action, every screen the video settled on, or given times — saved as JPEGs, with a numbered sheet of them |
| `report` | each step's expected, seen, verdict (`pass`, `fail`, `blocked`) and frame; refused without a frame for a pass or a fail |
| `stop` | saves a recording still running and shuts the device down |

A report goes on the ticket's record and into the member's conversation as a note (its first line
in the person's language, then a line a step, then the key frames as pictures). The desktop draws
it as a card — verdict, steps, frame thumbnails that open big, and **Play recording**, which opens
the video on the Mac that runs the squad; every other screen shows the same note as text and
pictures.

Who gets it: QA and mobile members, on a ticket, in a squad with a mobile app, in any mode but
read-only. Their calls are allowed without asking, since they only reach the ticket's own device.
Codex members get the same server, with a 10-minute tool timeout (booting a device takes a minute
or two).

## What counts as a mobile app

Read from the projects' files, three folders deep, skipping dependencies and build output
(`phone::detect`):

- **iOS**: an `*.xcodeproj` or `*.xcworkspace` — at the root, under `ios/` (React Native, Expo after
  prebuild, Flutter), or in a monorepo's app folder. Only on a Mac.
- **Android**: a Gradle module applying `com.android.application`, or an `AndroidManifest.xml` in a
  subfolder.
- **Both**: an Expo app (`app.json` with `expo`, or `expo`/`react-native` in `package.json`), or a
  Flutter app (`pubspec.yaml` naming `flutter`).

## Setup on the person's Mac

### iOS

- **Xcode** with an iOS simulator runtime (Xcode › Settings › Components). equipe never downloads a
  runtime or an Xcode component; a member without one is told to ask the person.
- **AXe** — to read the accessibility tree and touch, type and press. `simctl` (part of Xcode)
  boots, installs, launches, takes screenshots and records, but it can't tap or read the screen.
  The first time a member needs it, equipe asks the person on a card ("installing a tool"): AXe
  1.8.0, MIT, about 8 MB, into `~/.equipe/tools/axe/1.8.0/`. On yes, equipe downloads the pinned
  release from GitHub, checks its SHA-256, and unpacks it there; nothing else on the Mac changes,
  and it needs no root, no Accessibility or Screen Recording permission, and no Simulator window.
  A "no" holds until the daemon restarts; members then work with screenshots and recordings only.
  An `axe` the person installed themselves (`brew install cameroncooke/axe/axe`) is used as it is.

Why AXe: of the ways to drive the iOS Simulator, it needs the least. It is one self-contained
download built on the simulator frameworks of Meta's idb (FBSimulatorControl), talking to the
simulator through Xcode's own private frameworks. **idb** itself needs a Python client and a
companion process; a **WebDriverAgent/XCUITest** runner must be built with `xcodebuild` for each
Xcode (minutes, and DerivedData on disk) and is meant for real devices; **simctl** alone has no
input or accessibility. AXe is validated against Xcode 26.

### Android

- The **Android SDK** with `platform-tools` (adb) and the **Emulator** (Android Studio installs
  both), found through `ANDROID_HOME`, `ANDROID_SDK_ROOT`, `~/Library/Android/sdk` or
  `~/Android/Sdk`.
- A **system image** for the Mac's processor (`arm64-v8a` on Apple silicon). equipe never downloads
  one; it uses the newest installed (`google_apis` first).

equipe writes the ticket's AVD itself (its `.ini` files, as `avdmanager` would — no Java needed),
boots it with `-no-window`, and drives it with `adb`: `screencap`, `uiautomator dump` for the
elements, `input` to touch and type, `screenrecord` to record (three minutes at most at a time,
Android's own limit). `input text` types ASCII only.

### Frames

Read with AVFoundation on a Mac, else `ffmpeg` (install it on Linux).

## The ticket's device

- **Named for the ticket and the member**: the simulator `<squad> T<ticket> <member> <model>`
  (`acme T12 qa iPhone 17 Pro`), the AVD `<squad>_T<ticket>_<member>`. The tools only ever reach
  that device — never the person's own simulators, one a member made by hand, or a real phone.
- **One booted device per member**: starting one for another ticket shuts the last one down.
- **Deleted with the ticket**: when a ticket is merged or cancelled, equipe puts its members'
  devices away (a recording still running is saved), deletes its simulators
  (`disk::clear_simulators`) and emulators (`disk::clear_avds`).

## Recordings

Kept in the squad's folder, which every member reads and the person can open:
`~/.equipe/squads/<squad>/home/recordings/T-<ticket>/` — `<flow>.mp4`, its marks
(`<flow>.marks.json`), its frames (`<flow>/NN-<seconds>s.jpg`) and its report
(`<flow>.report.json`). They are kept out of the folder's git history (`.git/info/exclude`), so a
video is never committed.

They go with their ticket: when a ticket of the squad is done with, the recordings of every
ticket that is done are deleted (`disk::sweep_recordings`) — except one an open ticket names, as a
bug filed with its clip does; that one stays until the bug is done too. `equipe disk` lists them
("phone recordings"), and gives back those of finished tickets.

## Not yet

- **A real phone.** The tools refuse anything but the ticket's simulator or emulator. Pairing the
  person's own iPhone or Android phone (WebDriverAgent signed with their team over USB, or adb over
  USB) would be an explicit step of theirs; it is left for later.
- Android's `input text` is ASCII only; other text is typed by tapping the on-screen keyboard.

## Checking it

- `cargo test -p equipe-core phone::` — the rules, against a fake simulator.
- `EQUIPE_PHONE_LIVE_APP=/path/to/Shop.app cargo test -p equipe-core --test phone_live -- --ignored
  --nocapture` — the whole loop on a real iOS Simulator: AXe installed from its pinned release, a
  simulator made, a flow recorded, framed, reported, and the simulator deleted.
- `qa-mobile-flow` in the quality gate (`docs/quality-gate.md`): a real QA member tests a small
  SwiftUI app with a planted bug.
