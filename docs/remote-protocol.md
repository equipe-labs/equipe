# equipe remote protocol (v1)

How a phone (the equipe app) reaches the equipe daemon on the person's machine, through the
equipe relay, end to end sealed. Three parties:

- **daemon** — `equipe daemon` on the person's Mac/Linux box. Holds the squads.
- **relay** — a Cloudflare Worker + Durable Objects run by equipe-labs (`relay/` in this repo).
  Passes sealed frames and sends pushes. Never holds a key that opens a frame.
- **device** — a paired phone running the equipe app.

Both the daemon and devices connect *out* to the relay over WebSockets; nothing listens on
the person's machine.

## Identities and secrets

| What | Who makes it | Where it lives |
|---|---|---|
| daemon id (32 hex chars) and daemon relay token | relay, on `POST /v1/daemons` | daemon: `~/.equipe/remote.json` (0600); relay: SHA-256 of the token |
| daemon key pair, X25519 | daemon | daemon only |
| pairing secret, 32 random bytes | daemon, per `equipe pair` | in the QR code; relay sees only the slot (below) |
| device id (32 hex chars) and device relay token (32 random bytes) | daemon, when it approves a pairing | device: Keychain / Keystore; relay: SHA-256 of the token |
| device key pair, X25519 | device | device only (Keychain / Android Keystore-wrapped) |

Encodings: hex is lowercase; `b64` is base64url without padding.

## Sealing (HPKE, RFC 9180)

Suite: KEM `DHKEM(X25519, HKDF-SHA256)` (0x0020), KDF `HKDF-SHA256` (0x0001),
AEAD `ChaCha20Poly1305` (0x0003). Single-shot: one HPKE setup per frame.

A sealed frame is `b64(enc || ciphertext)`, `enc` being the 32-byte encapsulated key.

| Frame | Mode | Recipient key | Sender key | PSK / psk_id | info |
|---|---|---|---|---|---|
| pairing request (device → daemon) | PSK (0x01) | daemon | — | pairing secret / `equipe-pair` | `equipe/1 pair-request` |
| pairing answer (daemon → device) | AuthPSK (0x03) | device | daemon | pairing secret / `equipe-pair` | `equipe/1 pair-answer` |
| device → daemon | Auth (0x02) | daemon | device | — | `equipe/1 device-to-daemon` |
| daemon → device | Auth (0x02) | device | daemon | — | `equipe/1 daemon-to-device` |

`aad` is the ASCII string `<daemon id>` for pairing frames and `<daemon id>:<device id>`
for the others.

Platform libraries: CryptoKit `HPKE` (iOS 17+), BouncyCastle `org.bouncycastle.crypto.hpke`
(Android), the `hpke` crate (daemon).

### What's inside a frame

Plaintext is UTF-8 JSON:

```json
{ "ts": 1760000000000, "seq": 42, "squad": "equipe-squad", "rpc": { "jsonrpc": "2.0", … } }
```

- `ts` — sender's clock, Unix ms. Frames more than 5 minutes off are dropped.
- `seq` — starts at 1 on each WebSocket connection and goes up by one per frame, each
  direction; a frame whose `seq` isn't above the last one seen on that connection is dropped.
  It may skip (only "above" is checked); a dropped frame doesn't move it. The daemon starts
  both directions over when the relay tells it the device connected or left (`presence`), so
  the app starts both over on each new device socket. The `data` of a push has `seq` 0: it
  travels outside the connection, and is only shown.
- `squad` — which squad the `rpc` is about (absent for `squads`).
- `rpc` — a JSON-RPC 2.0 message of the daemon's own protocol (`crates/protocol/src/rpc.rs`):
  the same `hello` / `command` / `event` the terminal console speaks. `id`s are integers.

### What a device asks, and what comes back

| `rpc` | answer |
|---|---|
| `{ "id", "method": "squads" }` (no `squad`) | `result`: `[{ "name", "projects": [<project names>] }]` |
| `{ "id", "method": "hello", "params": { "version", "squad" } }`, `squad` set too | `result`: `{ "version", "snapshot" }`; from then on the squad's `event`s, with `squad` set. A device may follow several squads; a second `hello` for one replaces the first. Error `-32602` "no such squad"; `-32000` when the daemon couldn't follow it |
| `{ "id", "method": "read_image", "params": { "squad", "path", "max"? } }`, `squad` set too | `result`: a member's picture — see "A member's pictures" |
| `{ "id", "method": "list_images", "params": { "squad", "folder", "max"? } }`, `squad` set too | `result`: the pictures in a folder a reply names — see "A member's pictures" |
| `{ "method": "command", "params": <Command> }` — a notification, no `id` | none: what it did shows in events. Dropped without a word when it isn't one a device may send, its squad isn't the frame's, or the frame's squad isn't followed |
| `{ "id", "method": "upload", "params": <chunk> }` (no `squad` needed) | `result`: `{ "received" }`, and once the whole file is in, `path` and `tag` — see [Attachments](#attachments) |
| `{ "method": "upload_cancel", "params": { "upload_id" } }` — a notification | none |
| anything else with an `id` | error `-32601` "not something a device may ask" |

A frame that doesn't open, is off by more than 5 minutes, repeats a `seq` or doesn't parse is
dropped without an answer.

A push's `data` carries an `approval_requested` event with `action` cut to 800 characters, a
`ticket_changed` event (to `review` or `waiting`) with the ticket's `body` left empty, or a
`questions_asked` event with each question cut to 200 characters, each option's `label` to 60
and its `description` left out (the snapshot has them whole).

### A member's questions

A member puts a decision to the person as a `questions_asked` event: `questions` is
`{ "id", "session", "member", "ticket"?, "asked_at", "questions": [{ "question", "header"?,
"options": [{ "label", "description"? }], "multi_select"? }] }` — 1 to 4 questions, 2 to 4 options
each, the recommended option first — or `"options": []`, a plain question the person answers in
words of their own (`other`) only. The snapshot lists the ones still waiting in `questions`
(absent when none). The phone answers with the command `answer_questions`
`{ "squad", "id", "answers": [{ "picked"?: [<option index>…], "other"?: "<their words>" }, …] }`,
one answer per question in order (`{}` skips one; one pick only unless `multi_select`), or
closes them unanswered with `dismiss_questions` `{ "squad", "id" }`. Either way every screen gets
`questions_closed` `{ "squad", "id", "answers"? }` — `answers` only when answered — and the
member gets the answers as the person's next message, which shows in its conversation as a
`user_message` starting `Answers to your question`.

### A member's pictures

A visual choice carries its pictures: a question may have `"images": [<path>…]` (what it
shows as a whole), and each option `"images": [<path>…]` — at most 6 and 4, absent when none.
A ticket handed in for review may carry `"images": [<path>…]` too (at most 12): the pictures
its member named, then the images its branch changed. All three are additive; an older
screen ignores them. A ticket in review may also carry `"design_qa": <ticket id>` while a
designer checks its screens on that ticket first (see *Design QA* in ARCHITECTURE.md): a
screen should show it as in design QA rather than as waiting for the person; it is absent
once that passes, and from an older daemon. A path is the picture's on the daemon's machine, already checked when the
member named it: a PNG, JPG, WebP, GIF or SVG inside the squad's folder or the member's
worktree. A screen that shows options as pictures should lay them out side by side and let the
person pick from them; the pick is the same `answer_questions` as ever.

The bytes come only through the daemon, with the request `read_image`
`{ "squad", "path", "max"? }` (`id` set, the frame's `squad` the same and followed):
`result` `{ "mime", "size", "width"?, "height"?, "data", "scaled"? }` — `data` base64 of the
picture, or of a copy no bigger than `max` pixels on its longer side (`scaled: true`, PNG when
it has transparency, else JPEG); `width`/`height` are the file's own (absent for an SVG, which
goes as it is, at most 1 MB). The daemon answers only for a path a waiting question set, one
of the squad's tickets or one of its design systems (an asset, see below) names now, that is
still an image file (by name and by its first bytes) inside the squad's folder, a ticket's
worktree or a design folder once links are resolved — no `..`, at most
30 MB read, a copy made past 6 MB — and refuses an SVG with a script, an embedded page or a link
to anything but itself. For a phone the daemon makes the copy at most 720 pixels on a side,
smaller again until it fits one relay frame. Errors: `-32002` (not following that squad),
`-32000` with why (not shown, gone, not a picture). The words say which: `no such file` (gone —
`no such file on its branch` for a ticket's, see below), `bigger than` (too large), anything
else (not shown, not a picture, unreadable); a screen should say it calmly in the person's
language and collapse a gallery whose pictures all failed into one line with a retry.

**A ticket whose worktree is gone.** Accepting a ticket removes its worktree even when the
merge to main fails ("accepted, not in main yet"), so its pictures are no longer on disk. The
daemon then reads each recorded path from git, in the ticket's project: from the ticket's
branch while it isn't merged, from main (or the project's checkout, when the file is there)
once it is — by the path relative to the worktree the ticket recorded, never a ref or a path a
screen names. The same checks hold on the blob: an image by name and by its first bytes, at
most 30 MB.

**Pictures in the conversation.** A reply, a note or the person's own message may embed
pictures with Markdown — `![평면도](/…/images/plan.png)`, a title after the path, a path
relative to where its member works, spaces in it as members write them — and may name a folder
of them (an absolute path ending in `/`, or in a code span). Members are told to show pictures
this way, never to ask the person to open a folder. `read_image` gives such a picture too, by
the absolute path a screen makes of it (`file://` and `%20` read, a relative one joined to the
member's working directory, or the squad's folder; no `..`) — only when a line the daemon
itself recorded for that squad, among its latest 400 that name a path, embeds it, and only
inside the squad's folder or a worktree a member spoke from. A screen shows it inline (the
desktop asks for 1280 pixels, the column's width at 2x) once the reply has finished
streaming. The pictures of a folder a reply names come with `list_images`
`{ "squad", "folder", "max"? }` (`id` set, the frame's `squad` the same and followed):
`result` `{ "folder", "images": [<path>…], "total" }` — the image files right in the folder (not
below it), by name, each passing the same checks, at most 12 (`max` fewer), `total` all of them
(at most 2000 looked at); each path is one `read_image` then gives. Given only for a folder
such a recorded line names, inside the same roots. Errors as for `read_image`. A daemon older
than this answers `-32601`; a screen then shows no strip.

### A change to the team

The PM proposes adding members of a role, or letting one go, as a `team_change_proposed`
event: `change` is `{ "id", "change": "add" | "remove", "role", "count"? (1 when absent),
"member"? (who leaves), "engine"?, "model"?, "effort"?, "reason", "unblocks"?: [<ticket>…],
"by", "session", "proposed_at", "busy_on"? (the ticket a member let go is on) }`. The snapshot
lists the ones still waiting in `team_changes` (absent when none). Only the person decides —
never an agent: the phone approves with the command `approve_team_change` `{ "squad", "id" }`
(the daemon then does what `/add` or `/close` does) or says no with `decline_team_change`
`{ "squad", "id" }`. Every screen then gets `team_change_closed` `{ "squad", "id", "approved",
"members"? }` — `members`: who joined, or left — also sent with `approved: false` when the
change became moot (its member left anyway). A push for a proposal carries the event with
`reason` cut to 800 characters, category `team`.

### Accepting in words, confirmed on a card

When the person says in words that they accept, merge or send back handed-in work ("전부 수락",
"T-1이랑 T-3 수락해줘", "머지까지 해줘"), the PM — or the screen they typed it on, reading their
own words — puts it to them on a card: an `accept_proposed` event whose `proposal` is `{ "id",
"verdict": "accept" | "merge" | "changes", "tickets": [<ticket>…], "note"? (to send back: what to
change, in their words), "summary"?, "by" (the member that proposed it, or `"you"` when a screen
raised it from the person's words), "session" (the conversation it shows in), "proposed_at" }`.
The snapshot lists the ones still waiting in `accepts` (absent when none). Their words are never
the acceptance, an agent's even less: only the person's own command is. The phone confirms with
`confirm_accept` `{ "squad", "id", "tickets": [{ "ticket", "merge"?: "merge" | "push" }…] }` —
the tickets they kept — and the daemon runs, for each, what the board's buttons send (`accept`,
then `merge` when asked; or `reopen` with the note). From a phone `merge` is dropped: the work is
accepted, and merging stays at the console. It says no with `decline_accept` `{ "squad", "id" }`.
Every screen then gets `accept_closed` `{ "squad", "id", "confirmed", "tickets"? }` — `tickets`:
the ones acted on — also sent with `confirmed: false` when the card became moot (its tickets were
decided otherwise). A phone doesn't raise cards (`propose_accept` is the screens' own). A push for
a proposal carries the event with `summary` and `note` cut to 800 characters, category `accept`.

### The disk

When the disk the daemon writes to runs short, every followed squad gets a `disk_level` event —
only when the level changes — and the snapshot carries the same `disk` while it is short (absent
when there is room):

```json
{ "type": "disk_level", "status": { "level": "critical", "free": 4200000000, "total": 494384795648, "path": "/Users/abel/.equipe" } }
```

`level` is `ok`, `low` (under 15 GB free, or a tenth of the disk if that is less), `critical`
(under 5 GB: equipe holds new code tickets back so builds don't fail) or `full` (under 1 GB, or a
write just failed for want of room). A level is left only with a fifth more room than it took to
enter it, so a disk at an edge says so once. `free` and `total` are bytes (show them in decimal
units, as macOS does: `4.2 GB`); `path` is a folder on that volume. Additive: an older app ignores
it. Going `critical` or `full` is also pushed, once each way down (category `disk`, collapse
`disk`): "Only 4.2 GB left on the disk" / "Builds may fail · free space in equipe on the Mac".

What uses the space, and cleaning it up, stay at the Mac's screens: a device may not send
`disk_scan` or `disk_clean`, and the `disk_report` and `disk_cleaned` events those answer are not
forwarded to it.

### The plans' usage

Every screen, whichever squad it follows, gets `plan_usage` `{ "usage": { "five_hour",
"seven_day" }, "engine": "claude" | "codex" }` when an engine reports its plan's usage: each
window `{ "used_percent": 0–100, "resets_at": <unix seconds> }` or `null`. Windows are known
by their length — five hours, seven days — never by the order an engine lists them in; a plan
without one (Codex Pro has no five-hour window) sends `null` there, and a window of any other
length is left out. Claude's comes with its members' turns; Codex's with theirs and, while a
squad has Codex members, every five minutes besides. A `plan_usage` without `engine` (from an
older daemon) is Claude's. The snapshot carries Claude's as `plan` and every engine's as
`plans`: `[{ "engine", "usage", "plan_type"? ("max", "pro", "plus"…), "at" (unix seconds when
read) }]` — absent from an older daemon's.

### Both engines' plans, and moving members between them

`plan_usage` `{ "usage": { "five_hour", "seven_day" }, "engine"? }` says one engine's plan as last
read — each window `{ "used_percent", "resets_at" }` or `null` — for every screen; without
`engine` (an older daemon) it is Claude's. The snapshot keeps Claude's in `plan` and every
engine's in `plans`: `[{ "engine", "usage", "plan_type"?, "at" }]`, `at` the Unix seconds it was
read (older is less sure). How the squad moves members as a plan runs low is the snapshot's
`engine_switching` — `"off"`, `"suggest"` (when absent) or `"auto"`, changed at a screen with
`set_engine_switching` `{ "squad", "mode" }` and told as `engine_switching_changed`
`{ "squad", "mode" }`.

A card is `engine_proposed` `{ "squad", "proposal" }`, its `proposal` `{ "id", "why": "shift" |
"drain" | "exhausted" | "step_down" | "return" | "setup", "engine"? (the plan it is about; none
for the setup), "window"?: "five_hour" | "weekly", "used_percent", "resets_at", "moves": [{
"member", "role", "from", "to", "model"?, "effort"?, "was_model"?, "done"? }], "applied"?,
"proposed_at" }`. The snapshot lists the cards still showing in `engine_proposals` (absent when
none). Sent again with the same `id` as each member moves (`done: true`). Not `applied`: the
person makes the moves with `apply_engine_proposal` `{ "squad", "id" }` or leaves things as they
are with `dismiss_engine_proposal` `{ "squad", "id" }`. `applied`: equipe made them on its own
(`auto`); `undo_engine_proposal` `{ "squad", "id" }` takes them back and `dismiss_engine_proposal`
closes the card. Either way every screen then gets `engine_proposal_closed` `{ "squad", "id",
"applied" }`. Members move only between turns, at their next ticket. A push goes only for an
engine out of its plan with its reset hours away (`why: "exhausted"`), category `engines`, once.
`show_engine_setup` `{ "squad" }` proposes the recommended setup for both engines again.

### The design system

The squad's design system (`docs/DESIGN-SYSTEM.md`) is asked for with the command `design`
`{ "squad" }`. Every screen following the squad then gets the event `design`
`{ "squad", "systems": [ … ], "folder" }` — and again whenever one of the folders changes, for as
long as any screen follows the squad. `systems` lists the squad's own first (no `project`), then each
project's that has one (`project`: its name), then the drafts tickets in review hand in (`draft`:
`{ "ticket", "title" }`) — empty when there is none yet, `folder` being where the squad's would be.
Each system is `{ "project"?, "dir", "draft"?, "modes"?: ["light", "dark"], "tokens"?: [{ "name",
"type", "value", "modes"?: { "<mode>": <value> }, "description"?, "alias"? }], "problems"?: [<line>],
"guidelines"?: "<markdown>", "assets"?: [{ "path", "name", "kind": "logo" | "icon" | "illustration" |
"component" | "other", "size" }], "stamp" }`. Token values are DTCG's JSON with every alias resolved
(`value` in the first mode, `modes` every mode's when they differ). An asset's picture is read with
`read_image` by its `path`. A phone may send `design`; nothing in it changes anything.

### Attachments

A phone attaches a photo, a file, a sound or a video the way a screen does: it puts the file
on the daemon's machine first, then sends a `submit` whose `text` carries the file's **tag**.
A file goes in pieces, each piece one ordinary sealed frame (`seq`, `ts`, the same keys), so
the relay passes it like any other and can't read it — no other channel, no storage at the
relay.

**Does this daemon take uploads?** The answer to `hello` carries `uploads` when it does:

```json
{ "version": "…", "snapshot": { … },
  "uploads": { "chunk": 393216, "image": 8388608, "file": 26214400, "video": 67108864, "transcript": 65536 } }
```

(bytes: the largest piece, the largest image, other file, video and transcript). A daemon
without it answers `upload` with `-32601`; the app then says the Mac's equipe needs an update.

**A piece** — `upload`, a request:

```jsonc
{ "jsonrpc": "2.0", "id": 7, "method": "upload", "params": {
    "upload_id": "9b2c4e1a-…",   // the phone's, 8–64 of A–Z a–z 0–9 - _ ; one per file
    "name": "IMG_0421.jpg",      // its name where it came from; folders are dropped
    "kind": "image",             // image | video | pdf | file (sounds are files: m4a, mp3, wav, aac, caf, ogg, opus, flac)
    "total": 612345,             // the whole file, bytes
    "offset": 0,                 // where this piece starts
    "data": "<base64, standard alphabet, padded; at most `chunk` bytes once decoded>",
    // only with the first piece (offset 0), all optional:
    "transcript": "…",           // what the phone heard in a sound or a video
    "duration": 31.4,            // a sound's or a video's length in seconds, when the phone knows it
    "pages": 12                  // a PDF's page count
} }
```

Pieces of one file go in order, one at a time (send the next once the last is answered); two
files may go at once, at most 4 per device. A piece of 384 KiB seals to about 700 KB, well
inside the relay's 1 MiB frame.

The answer:

```jsonc
{ "result": { "received": 393216 } }                       // more to come
{ "result": { "received": 612345,                           // the last piece: the file is kept
              "path": "/Users/abel/.equipe/uploads/1760000000000-IMG_0421.jpg",
              "tag": "[image: /Users/abel/.equipe/uploads/1760000000000-IMG_0421.jpg]" } }
```

- `received` is how much of the file the daemon has, in order. A piece it already has (the same
  `upload_id` and an `offset` below `received`) is answered with `received` and changes nothing,
  so a piece sent again after a lost answer is harmless; the last piece sent again gets the same
  whole answer, for an hour.
- An `offset` above `received` — or a piece for an `upload_id` the daemon doesn't know, past 0
  (it was swept, or the daemon restarted) — is `-32602` "send from received" with
  `data: { "received" }`: the phone carries on from there (0: from the start).
- **Resuming.** A connection that drops halfway loses nothing: the pieces are kept by
  `upload_id`, not by connection. After the next `hello` the phone sends the piece it was on
  again; the answer says where to go on from. A file no piece has reached for an hour is thrown
  away.
- **Cancelling**: `upload_cancel { upload_id }`, a notification: what came of that file is thrown
  away. Unknown ids are ignored.
- The daemon checks for itself, never trusting the phone: the `kind`'s extensions (an image is
  png, jpg, jpeg, gif or webp; a PDF ends in .pdf), the size for the kind, the piece's size, that
  `total`, `name` and `kind` stay what the first piece said. Refusals:

| error | when | `data` |
|---|---|---|
| `-32602` "too large" | `total` over the kind's limit, or a `transcript` over its limit | `{ "limit" }` |
| `-32602` "piece too large" | a piece over `chunk` bytes, or one going past `total` | `{ "limit" }` |
| `-32602` "send from received" | `offset` isn't where the daemon is | `{ "received" }` |
| `-32602` "not the same upload" | `name`, `kind` or `total` differ from the first piece's | |
| `-32602` (why) | the name doesn't fit the kind, a missing field, data not base64 | |
| `-32000` "too many uploads at once" | a fifth file under way from one device | |
| `-32000` (why) | the daemon couldn't write it | |

**What the file becomes.** It is kept like a screen's upload: `~/.equipe/uploads/`, 0600 in a
0700 folder, named by the daemon (`<ms>-<tidied name>.<ext>`), swept after 7 days. Pieces wait
in `uploads/.partial/`, which nothing reads back. Then, by kind:

| kind | kept | `tag` |
|---|---|---|
| image | as sent | `[image: <path>]` |
| pdf | as sent | `[pdf: <path> · 12 pages]` (`pages` as the phone said) |
| a sound (file) | as sent, and the `transcript` as `<name>-transcript.txt` beside it | `[audio: <path> · 0:31] [file: <transcript> · 2 lines]` |
| other file | as sent | `[file: <path> · 3 lines]` (lines counted by the daemon, for UTF-8 text up to 8 MB) or `[file: <path>]` |
| video | as sent; its frames — 4 to 12, one per ten seconds, from the middle of equal parts, 1568 px on the long side — and the `transcript`, as files beside it | `[video: <path> · 0:42 · 1920×1080 · frames: <f1>@0:05, …] [file: <transcript> · 3 lines]` |

A member can't watch a video or hear a sound: it reads a video's frames and a transcript. The
frames are made on the daemon's machine with `ffmpeg` (looked for in `/opt/homebrew/bin`,
`/usr/local/bin`, `/usr/bin`). Without it the video is kept and tagged without frames, and the
last answer says so — `"notice": "no_ffmpeg"` — for the app to tell the person
("Install ffmpeg on your Mac (`brew install ffmpeg`) to send what a video shows"); the daemon's
log says it too. A sound sent without `transcript` is tagged alone.

**Pointing at it.** The phone puts each finished `tag`, followed by a space, after what was
typed — the desktop's `attach::message`: `"<typed> <tag> <tag>"`, or the tags alone — and sends
that as `submit`'s `text`. Tags are text: the member reads the files by their paths as it reads
a screen's (`crates/protocol/src/tags.rs` is the grammar). Uploading gives a phone no other
command, and doesn't send anything: a file nobody's message points at is only swept.

### A member's conversation

Each session's events (`params` carry `session`) make its conversation. Besides `user_message`,
`message_queued`, `text_delta`, `tool_use`, `tool_result`, `turn_ended`, `notice` and `error`:

- `steer_queued` `{ "session", "text", "project"? }` — a `submit` sent while the member works,
  with an engine that takes it mid-turn (Claude Code): it reads it at its next step, without
  stopping. It confirms the `submit` as `user_message` and `message_queued` do; `project` is the
  one the person was looking at (absent: the whole squad). Not a line yet.
- `steered` `{ "session", "text" }` — the member read it: a user line here, in the middle of the
  turn. A steer not read before the turn ends becomes a memo (`message_queued`) and is
  delivered after it.
  `memo_withdrawn` `{ "session", "text" }` takes back a memo or a steer not read yet.
- `thread` `{ "session", "project" }` — what follows is about `project` (`null`: the whole
  squad): a message sent looking at it, news of its ticket, a ticket's work. Never shown itself;
  a screen shows one project's slice.
- `model_known` `{ "session", "model" }` — the engine named its model, or the person changed it.
- `branch_landed` `{ "session", "branch", "base", "stat" }` — an editing session closed with
  changes committed to `branch`, made from `base`; `stat` is `git diff --stat`.
- `nothing_changed` `{ "session" }` — an editing session closed without changes; no branch kept.
- `error` `{ "session"?, "message", "next"? }` — `next` is the command that fixes it, when
  there is one.
- `compacting` `{ "session", "member"?, "project"?, "trigger", "started_at", "estimate_ms" }` —
  the member's conversation is being compacted (summarized to make room): by its engine
  mid-turn, or by equipe as a desk conversation starts afresh. `trigger` is `auto` (the
  engine's context nearly full, or equipe's desk conversation full or cold) or `manual` (the
  person's `/compact`). `started_at` is unix milliseconds. `estimate_ms` is how long the last
  compactions of the same engine and model took (the longest of the last five; two minutes
  before there are any) — **a guess, not progress**: a screen may show time spent against it,
  never a percentage. `project` absent: the whole squad's conversation.
- `compacted` `{ "session", "member"?, "project"?, "trigger", "took_ms" }` — done; the member
  works on.
- `compact_failed` `{ "session", "member"?, "project"?, "trigger", "message" }` — it failed, or
  its turn ended first (then just before `turn_ended`).
  What the person sends meanwhile is handled as at any moment of a turn (`steer_queued`, read
  after the compaction, or a memo after the turn): nothing is lost.

The snapshot's sessions carry `recent`, the latest lines oldest first, each `{ "kind", … }`:
`user` / `reply` `{ "text" }`, `tool` `{ "tool", "target", "outcome"? }`, `note` `{ "text" }`,
`error` `{ "message", "next" }`, `landed` `{ "branch", "stat" }` and `thread` `{ "project" }`
(lines before any `thread` are about the whole squad). Beside `queued` (memos, and `queued_in`:
the project each was left about), `steering` lists the steers not read yet, and `steering_in`
the project each was said about (`null`: the whole squad; absent when there are none). A
session whose conversation is being compacted carries `compacting`
`{ "trigger", "started_at", "estimate_ms" }` (absent otherwise), for a screen attaching meanwhile.

**Folding.** Events become lines the way `crates/protocol/src/transcript.rs` folds them, the
same in the journal, the snapshot and every screen, so a screen that attaches later sees what a
screen that watched saw:

- `text_delta` joins the open reply, or opens one.
- A reply is closed by the next line, by `turn_ended`, `work_ended` or `session_closed`.
  Events that make no line and may come mid-sentence — `steer_queued`, `message_queued`,
  `queue_sent`, `memo_withdrawn`, `thinking`, `tool_result`, `output` (a token count),
  `model_known`, `subagent_*`, `compacting`, `compacted`, `compact_failed`, `estimated`,
  `suggestion`, `browser_frame` — leave it open: a steer sent while the member writes
  doesn't cut its answer; where it is read (`steered`) the reply so far ends and the steer
  follows it.
- Lines: `user_message` and `steered` → `user`; `notice` → `note`; `thread` → `thread`;
  `tool_use` → `tool` (its `tool_result` sets `outcome` on the latest open call of the same
  `tool` and `target`); `error` → `error`; `branch_landed` → `landed`; `nothing_changed` → note
  `No files changed, so no branch was kept.`; `turn_ended` `interrupted` → note `Stopped.`,
  `incomplete` → note `The engine ended early.`; `work_started` → note `── T-<n> · <title>`
  (and ` — on branch <branch>`). Notes are in English; a screen shows them in the person's
  language.
- A memo is not a line until it is delivered, as a `user_message` (memos left meanwhile arrive
  together). At most 500 lines are kept.

### Conformance vectors

`docs/remote-vectors/` holds frames sealed with fixed keys and fixed ephemeral keys, QR codes
good and bad, one pairing start to end, `seq` and `ts` cases, the JSON the daemon sends,
screen by screen, and a phone's files sent a piece at a time (`uploads.json`) — all made by the daemon's code and checked against it:
`cargo test -p equipe vectors` (`EQUIPE_WRITE_VECTORS=1` writes them again). The equipe app
keeps a copy in its `docs/vectors/`.

## The QR code

```
equipe://pair?r=<relay base URL, urlencoded>&d=<daemon id>&k=<b64 daemon public key>&s=<b64 pairing secret>&n=<machine name, urlencoded>
```

Every parameter is there once, in any order; the app ignores ones it doesn't know. Values are
percent-encoded UTF-8: the daemon escapes every byte but `A-Z a-z 0-9 - _ . ~` and never
writes `+`. `r` is `https://` (or `http://localhost` / `127.0.0.1`, for development), `d` 32
lowercase hex, `k` and `s` unpadded base64url of 32 bytes.

The pairing **slot** is the first 32 hex chars of `SHA-256(pairing secret)`.

## Pairing

1. `equipe pair` registers the daemon with the relay if it isn't yet, opens a slot
   (`pair_open`, 5 minutes) and shows the QR code.
2. The app scans it, makes its key pair and connects to
   `wss://<relay>/v1/daemons/<daemon id>/pair/<slot>`, then sends one sealed pairing request:
   ```json
   { "type": "pair_request", "name": "Abel's iPhone", "platform": "ios", "key": "<b64 device public key>" }
   ```
   The daemon keeps the first 60 characters of `name` (control characters dropped) and the
   first 20 ASCII letters and digits of `platform`; a request whose `key` isn't 32 bytes is
   ignored.
3. The daemon asks the person at the terminal: `Pair Abel's iPhone (ios)? y/n`.
4. On yes, it makes the device id and relay token, registers them with the relay
   (`device_add`), keeps the device, and answers on the slot:
   ```json
   { "type": "paired", "device": "<device id>", "token": "<b64 device relay token>", "name": "abel-macmini" }
   ```
   On no: `{ "type": "refused" }`. The slot closes either way.
5. The app keeps the device id, token and keys, and connects as a device.

`equipe devices` lists paired devices; `equipe devices remove <name>` forgets one and has the
relay drop it (`device_remove`) — its socket is closed at once.

### From a screen (the desktop app)

A screen pairs a phone over the daemon's own protocol, so a window on another Mac pairs the
phone with the daemon's machine, not its own. It sends `pair_start`, `pair_answer { approve }`,
`pair_cancel`, `devices` and `unpair { device }` as commands; the gateway answers that screen
alone with `pair_offer { url, seconds }`, `pair_asked { name, platform }`, `paired { name }`,
`pair_ended { why, message }` (`why`: `refused`, `expired`, `busy`, `relay`, `unavailable`,
`failed`) and `devices { devices, relay, connected }` events. The runtime never sees them, and a
phone may not send them (they're dropped like any command a device may not send).

The gateway carries them to the same local socket `equipe pair` uses (`~/.equipe/remote.sock`,
one JSON object per line: `{"op":"pair"}`, `answer`, `cancel`, `devices`, `remove`), so the
terminal and the app pair the one same way. A code belongs to the line that opened it: another
line asking gets `{"error", "why":"busy"}` and can't close it; the same line asking again closes
its old slot and opens a new one; the line closing (the screen leaving, Ctrl-C) closes it.
Answers to `devices` and `remove` carry `"op"`; a device row is `{ id, name, platform, added,
online, seen? }`, `seen` being when it last connected or left (kept in `remote.json`). The code
reaches the screen only in `pair_offer` and is never logged or kept on either side.

### The engines (AI platforms)

A screen asks where each engine stands on the daemon's machine with the command `engines`; every
screen (a phone too) then gets `engines { engines: [{ engine, state, plan?, per_token?, detail?,
members?, signing_in?, legacy? }] }` — `state`: `signed_in`, `signed_out`, `not_installed`, or
`unknown` (also what an older screen reads for a state it doesn't know); `members`: `[{ squad,
member }]` on that engine now; `legacy`: equipe's own older Codex login (`equipe login codex`) is
kept beside the tool's. It is told again after every sign-in and sign-out. Entries for engines a
screen doesn't know are left out, not an error. Each engine is the person's own command-line tool
with its own login, which equipe asks about (`claude auth status`, `codex login status`) and never
reads.

A screen signs Codex in with `sign_in { engine: "codex", device? }`: the daemon runs `codex login`
on its own machine — the browser opens there — or, with `device: true` (a window on another Mac),
`codex login --device-auth`. Every screen gets `sign_in { engine, step }`, `step` being
`{ kind: "browser", url? }` (the page to open by hand if the browser didn't), `{ kind:
"device_code", url, code }`, `{ kind: "done" }`, `{ kind: "failed", why, detail? }` (`why`:
`not_installed`, `device_code_off`, `expired`, `unsupported`, `failed`) or `{ kind: "cancelled" }`
(after `sign_in_cancel { engine }`). `sign_out { engine }` runs `codex logout`. Claude Code signs in
on its own (`claude auth login`, in a terminal on the daemon's machine): `sign_in` for it fails
with `unsupported`.

A phone may send `engines`, never `sign_in`, `sign_in_cancel` or `sign_out` (dropped like any
command a device may not send), and never gets `sign_in` events: a one-time code signs the
daemon's machine in to whoever enters it.

### A phone forgets itself

A phone can also forget itself ("Erase this phone" in the app, for account deletion): it sends
`unpair` on its device socket. The relay forgets its token and push token, answers `unpaired`
and closes the socket (1000); a connection with the old token gets `401`. It tells the daemon
`device_unpaired` — at once, and again on each daemon connection until the daemon answers
`device_remove` — and the daemon forgets the device as `equipe devices remove` would. An app
that gets no `unpaired` within a few seconds (an older relay answers `error`) erases itself
anyway and tells the person to run `equipe devices remove` on the Mac.

## Demo pairing (store review)

App Review and Google Play reviewers have no Mac to pair with. For them equipe-labs runs a
**demo daemon** — `equipe demo serve` — whose code is printed in the review notes and never
expires. Its rules, and why they stay away from the person's own daemon:

- **A process of its own.** `equipe demo serve` keeps its own home (`$EQUIPE_HOME/demo` unless
  `--home` says otherwise), registers with the relay under a daemon id and key pair of its own,
  and never connects to `equipe daemon`: it can't see or reach anyone's squads, and runs no
  agent and no engine. What a phone gets is one squad, `demo`, played from a script — a board,
  members, a request for permission, the PM answering — kept apart for each paired phone, so
  reviewers don't see each other. Nothing a phone sends leaves its own copy of the script.
- **A standing code.** Its pairing secret is made once and kept (`demo.json`, 0600), so the QR
  code and its `equipe://pair?…` address stay the same across restarts, with `n=equipe demo`.
  The relay is the same and unchanged: the demo sends `pair_open` for its slot every 4 minutes
  (10 minutes ahead, the relay's limit) and again right after each answer, since the relay
  closes a slot once it is answered.
- **Answered without asking.** Every well-formed pairing request on the standing slot is
  answered `paired` at once — there is no person at a terminal. The relay still lets one phone
  at a time on a slot: one that finds it taken (`409`) tries again a moment later.
- **Bounded.** At most 200 phones are kept; past that the oldest is forgotten (`device_remove`).
  A phone may do what any phone may (below), against its own copy of the script only.
- **`equipe demo reset`** forgets every demo phone (the relay too) and starts the script over;
  `--new-code` also makes a new secret — the old address stops working, so the review notes
  must be updated.

Running it: [demo-daemon.md](demo-daemon.md).

`equipe pair` is untouched by any of this: a code works for 5 minutes, for one phone, after the
person's yes at the terminal, and `equipe daemon` never opens a standing slot.

## Relay API

Base URL: the relay's, as deployed (the `equipe-relay` Worker on Cloudflare; `EQUIPE_RELAY` overrides it).

### HTTP

- `POST /v1/daemons` → `201 { "id": "<daemon id>", "token": "<b64 daemon token>" }`.
  Rate-limited per IP.

### Daemon socket: `GET /v1/daemons/<id>/connect` (WebSocket), `Authorization: Bearer <daemon token>`

One per daemon; a new one replaces the old. Text frames, JSON:

daemon → relay

| `type` | fields | |
|---|---|---|
| `send` | `device`, `data` | a sealed frame for a device |
| `pair_open` | `slot`, `expires` (Unix ms, ≤ 10 min ahead) | |
| `pair_send` | `slot`, `data` | the sealed pairing answer |
| `pair_close` | `slot` | |
| `device_add` | `device`, `token_hash` (hex SHA-256 of the device token) | |
| `device_remove` | `device` | closes its socket, forgets its push token; also the answer to `device_unpaired` |
| `push` | `device`, `title`, `body`, `category`, `data`, `collapse?` | see Push |

relay → daemon

| `type` | fields | |
|---|---|---|
| `recv` | `device`, `data` | a sealed frame from a device |
| `pair_opened` | `slot` | the slot is open: the daemon shows the QR code only now |
| `pair_recv` | `slot`, `data` | a sealed pairing request |
| `presence` | `device`, `online` | a device connected or left; on connecting, the daemon gets one for each device already there |
| `push_result` | `device`, `ok`, `error?` | |
| `device_unpaired` | `device` | the phone erased itself (`unpair`); sent again on each connection until the daemon answers `device_remove` |
| `error` | `message` | |

### Device socket: `GET /v1/daemons/<id>/devices/<device>/connect` (WebSocket), `Authorization: Bearer <device token>`

device → relay: `{ "type": "send", "data" }`; `{ "type": "push_token", "platform": "apns" | "fcm", "token", "sandbox"? }`;
`{ "type": "unpair" }` (forget this device — see Pairing)

relay → device: `{ "type": "recv", "data" }`; `{ "type": "presence", "online" }` (the daemon's);
`{ "type": "unpaired" }` (the answer to `unpair`, just before the socket closes)

A device's `send` is refused with `error` "frame is too large" when the `recv` the daemon would
get — its `data` wrapped with the device id — passes 1 MiB, and with `error` "slow down"
(`retry_ms`: when to try again) once a device has sent more than 96 MiB of `data` in a minute;
either frame is dropped, never cut. A `send` while the daemon is away is dropped too (the device
has `presence` `online: false`): an upload waits and resumes.

### Pairing socket: `GET /v1/daemons/<id>/pair/<slot>` (WebSocket), no auth

Only while the slot is open, and for one connection. device → relay `{ "type": "send", "data" }`;
relay → device `{ "type": "recv", "data" }`.

## Push

The daemon asks for a push when something needs the person and the device isn't connected:
a request for permission (`approval`), work to review (`review`), a ticket waiting on them
(`waiting`), a decision to make (`decision`), a member's answer to them (`message`), the disk
getting critical or full (`disk`).

`title`, `body` and `category` are plain strings of at most 200 characters. `title` and `body` are what the lock screen shows — who needs what, never the command or
content ("frontend asks to run a command"). `data` is a sealed frame (daemon → device) the
app opens on the phone for detail and actions: iOS in a Notification Service Extension
(`mutable-content: 1`, `category` for the Allow / Decline actions), Android from an FCM data
message.

The relay sends with APNs (token auth, `.p8` key) and FCM HTTP v1 (service account), from
its secrets. Without them configured, a push answers `push_result` `ok: false`.

## What a device may do

The daemon follows a device like one more screen, with less reach than the terminal:

- **may:** `hello`, `squads`, `upload`, `upload_cancel` (files only: see Attachments), `read_image` and
  `list_images` (a member's pictures, of a squad it follows: see A member's pictures), and the commands `submit`, `interrupt`, `send_now`, `accept`, `reopen`,
  `resume`, `cancel`, `decide`, `answer_questions`, `dismiss_questions`, `approve_team_change`,
  `decline_team_change`, `confirm_accept` (accepting only: see Accepting in words), `decline_accept`,
  `apply_engine_proposal`, `dismiss_engine_proposal`, `undo_engine_proposal`, `focus`, `refresh`,
  `design`, `engines` (where the engines stand; never signing one in or out).
- `submit` and `answer_questions` may say what the message is about: `about` — the project the
  phone shows, or `null` for the whole squad. Left out, the daemon takes the project in focus
  (the last `focus` any screen sent). Said while the member works, a message steers it
  (`steer_queued`, with its `project`); `send_now` stops the turn and sends what waits — the
  `texts` first, in that order — as one message (`queue_sent`, then the turn's end).
- **may not:** `start`, `close`, `merge`, `set_mode`, `scale`, `show_browser`,
  `set_engine_switching`, `show_engine_setup`, `propose_accept` — nor the person's
  own hand on the board: `create_ticket`, `edit_ticket`, `edit_tickets`, `move_ticket`,
  `comment_ticket`, `ticket_history` (the phone answers what the board asks; planning stays at a
  screen for now).
- `decide` with `always` counts as `allow`: from a phone a request is allowed once.

The app asks for Face ID / fingerprint before it sends a `decide`.

### Answering a waiting ticket

A `waiting` ticket carries what it asks as `asks` (one string an item). `resume`'s `note` is
the person's words — or, to answer item by item as the desktop card and the terminal do, the
message `equipe_protocol::asks::answer_message` writes (every ask numbered, each with words,
`done ✓`, `do it yourself …`, `later …`, `your call …`, `the person doesn't know …` or
`— (not yet …)`, a `Note:` under it, `From the person:` last). The daemon reads that message
back: what it leaves open (not yet, or later) stays on the ticket as `remaining` — additive,
left out when empty — and is asked again the next time the member waits on the person.
