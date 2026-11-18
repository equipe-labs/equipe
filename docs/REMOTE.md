# Working from another machine

The daemon runs on one Mac — often a Mac mini that stays on — and the person
follows a squad from wherever they are: the equipe app on that same Mac, the
app on another Mac (over SSH, see docs/DESKTOP.md, "Remote"), a terminal over
SSH, or a phone through the relay. Whatever equipe shows the person, and
whatever a member asks them to open, run or approve, has to reach the screen
they are actually at. equipe works that out by itself; nobody sets a "remote
mode".

## One notion: where the person's screen is

`crates/core/src/screens.rs` keeps the screens that follow each squad and
where each one is:

| Place | Who | How the daemon knows |
| --- | --- | --- |
| Here | the app or a terminal on the daemon's own Mac | the default |
| Elsewhere | the app on another Mac, a terminal over SSH | `hello { remote: true }`, or the connection came through `equipe attach --stdio`, which sends `bridged` as its first line |
| Phone | the equipe app on a phone, through the relay | `hello { phone: true }` |

A screen takes a seat when it says `hello` and leaves with its connection. A
squad's [`Following`] answers two questions:

- **`can_show()`** — does a screen that shows pages beside the conversation
  (the app's live preview and browser panel) follow the squad right now? A
  terminal or a phone doesn't.
- **`remote()`** — does the person look from another machine? True when a
  screen elsewhere (or a phone) follows the squad — with one on the daemon's
  Mac too, nothing of that Mac is surely theirs — or when none follows and the
  last one that did was elsewhere (the laptop lid closed for a minute).

The bridge marks its connections itself, so an older app on another Mac still
counts as remote. The desktop app also says so in its `hello` (`remote`), and
`equipe` in a terminal over SSH does too.

## What uses it

### Pages a member shows (`equipe design session show`)

A designer shows a page beside a design session with `equipe design session
show <file.html | folder | http://localhost:port/path> [--variant A|B|C]`. A
saved HTML file used to reach the person's screen as a `file:///Users/…`
address, which only the daemon's Mac can open — the live preview and the
variants side by side never opened it, locally or from another Mac.

Now the daemon reads the command's output as it ends
(`crates/core/src/runtime/shown.rs`) and, for a saved page:

1. checks it is where the member works (its worktree, its working folder,
   the squad's folder);
2. serves it on the daemon's page server (`crates/core/src/prototype.rs`,
   `127.0.0.1` with a token, `Policy::Session`): its design folder — so
   `../tokens.css` resolves — or its own folder, with the web's fonts and
   pictures allowed and framing allowed (the variants side by side are frames
   of one page);
3. answers the screen's `prototype_url` for it, so the app swaps the
   `file://` address for `http://127.0.0.1:<port>/…`. The live preview
   forwards that port over the window's SSH link, like any member's dev
   server, and all variants share the port, so side by side works.

This happens before the screen hears of the command, so the app never waits
on it. The page is served the same way when the app is on the daemon's own
Mac: a `file://` page doesn't open in the live preview or side by side there
either.

When **no screen that can show a page follows the squad**, the member hears
it at once, mid-turn: what it showed reached nobody, and it must not tell the
person it's on their screen. A saved page outside where the member works, or
one gone since, is said the same way. The command itself can't ask the
daemon — inside an engine's sandbox a member's commands don't reach its
socket — so this is read from what the command printed, as the screen reads
it.

When the person picks a served variant with "Use this", the implementation
ticket gets the page's file back, not the server's address, which ends with
the daemon.

### What members are told

The first turn after the person's screen moves to another machine opens with
a note (`screens::remote_note`): they can't open the squad Mac's files or its
`localhost`, see its screen, or type in its terminal. So members never hand
them `file://` links or local paths to open; they show pages with `equipe
design session show`, pictures with their board's tools, and a file's
contents in the reply; a command they suggest says where it runs ("on
<Mac>" or "on your own computer"); and what only the squad Mac's own screen
can do — a Keychain or macOS permission dialog, a native window — is said
plainly: one approval on <Mac>'s screen, through Screen Sharing. A sign-in or
CAPTCHA in the members' browser goes to the browser pane of the person's app.
When the person is back on the daemon's Mac, members hear that once too. The
note comes only when where the person looks from changes, so it costs no
context turn after turn.

`<Mac>` is the daemon machine's computer name (`screens::machine_name`), the
same name every "on <Mac>" line uses.

### Things that open on the squad Mac's own screen

A member's `show_my_browser` may open a normal Chrome window (a sign-in) or
the members' browser on the daemon's Mac. While the person follows from
another machine, both the member and the person hear where it is and how to
reach it (`notice.on_squad_mac`): Screen Sharing to that Mac. A permission
card already names the Mac and offers no button that would open the wrong one
(`turn.permission_*_far` in the app).

### Links and paths in the app

- A link to the daemon's `localhost` in a reply opens in the live preview,
  forwarded over SSH, instead of this Mac's own browser, where `localhost` is
  another machine (`views::preview::remote_page`).
- A path in a reply is fetched over the window's SSH link into a private temp
  file and opened here, instead of looking for it on this Mac's disk. A
  document already opened beside the conversation from either Mac.
- Pictures, attachments, reference cards, a decision's live prototype and the
  live preview already came through the daemon.

## Not yet

- A forward that fails on the person's screen (the live preview's SSH link)
  is shown there, but the member isn't told.
- The page server's port and tokens change when the daemon restarts: a page
  shown before then is shown again by the designer to come back.
- A terminal over SSH counts as remote, but nothing yet shows it the design
  session's page; it relies on the member's words.
