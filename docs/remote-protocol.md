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
| `{ "method": "command", "params": <Command> }` — a notification, no `id` | none: what it did shows in events. Dropped without a word when it isn't one a device may send, its squad isn't the frame's, or the frame's squad isn't followed |
| anything else with an `id` | error `-32601` "not something a device may ask" |

A frame that doesn't open, is off by more than 5 minutes, repeats a `seq` or doesn't parse is
dropped without an answer.

A push's `data` carries an `approval_requested` event with `action` cut to 800 characters, or a
`ticket_changed` event (to `review` or `waiting`) with the ticket's `body` left empty.

### Conformance vectors

`docs/remote-vectors/` holds frames sealed with fixed keys and fixed ephemeral keys, QR codes
good and bad, one pairing start to end, `seq` and `ts` cases, and the JSON the daemon sends,
screen by screen — all made by the daemon's code and checked against it:
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
| `device_remove` | `device` | closes its socket, forgets its push token |
| `push` | `device`, `title`, `body`, `category`, `data`, `collapse?` | see Push |

relay → daemon

| `type` | fields | |
|---|---|---|
| `recv` | `device`, `data` | a sealed frame from a device |
| `pair_opened` | `slot` | the slot is open: the daemon shows the QR code only now |
| `pair_recv` | `slot`, `data` | a sealed pairing request |
| `presence` | `device`, `online` | a device connected or left; on connecting, the daemon gets one for each device already there |
| `push_result` | `device`, `ok`, `error?` | |
| `error` | `message` | |

### Device socket: `GET /v1/daemons/<id>/devices/<device>/connect` (WebSocket), `Authorization: Bearer <device token>`

device → relay: `{ "type": "send", "data" }`; `{ "type": "push_token", "platform": "apns" | "fcm", "token", "sandbox"? }`

relay → device: `{ "type": "recv", "data" }`; `{ "type": "presence", "online" }` (the daemon's)

### Pairing socket: `GET /v1/daemons/<id>/pair/<slot>` (WebSocket), no auth

Only while the slot is open, and for one connection. device → relay `{ "type": "send", "data" }`;
relay → device `{ "type": "recv", "data" }`.

## Push

The daemon asks for a push when something needs the person and the device isn't connected:
a request for permission (`approval`), work to review (`review`), a ticket waiting on them
(`waiting`), a member's answer to them (`message`).

`title`, `body` and `category` are plain strings of at most 200 characters. `title` and `body` are what the lock screen shows — who needs what, never the command or
content ("frontend asks to run a command"). `data` is a sealed frame (daemon → device) the
app opens on the phone for detail and actions: iOS in a Notification Service Extension
(`mutable-content: 1`, `category` for the Allow / Decline actions), Android from an FCM data
message.

The relay sends with APNs (token auth, `.p8` key) and FCM HTTP v1 (service account), from
its secrets. Without them configured, a push answers `push_result` `ok: false`.

## What a device may do

The daemon follows a device like one more screen, with less reach than the terminal:

- **may:** `hello`, `squads`, and the commands `submit`, `interrupt`, `accept`, `reopen`,
  `resume`, `cancel`, `decide`, `focus`, `refresh`.
- **may not:** `start`, `close`, `merge`, `set_mode`, `scale`, `show_browser`.
- `decide` with `always` counts as `allow`: from a phone a request is allowed once.

The app asks for Face ID / fingerprint before it sends a `decide`.
