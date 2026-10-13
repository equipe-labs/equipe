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
- `squad` — which squad the `rpc` is about (absent for `squads`).
- `rpc` — a JSON-RPC 2.0 message of the daemon's own protocol (`crates/protocol/src/rpc.rs`):
  the same `hello` / `command` / `event` the terminal console speaks.

## The QR code

```
equipe://pair?r=<relay base URL, urlencoded>&d=<daemon id>&k=<b64 daemon public key>&s=<b64 pairing secret>&n=<machine name, urlencoded>
```

The pairing **slot** is the first 32 hex chars of `SHA-256(pairing secret)`.

## Pairing

1. `equipe pair` registers the daemon with the relay if it isn't yet, opens a slot
   (`pair_open`, 5 minutes) and shows the QR code.
2. The app scans it, makes its key pair and connects to
   `wss://<relay>/v1/daemons/<daemon id>/pair/<slot>`, then sends one sealed pairing request:
   ```json
   { "type": "pair_request", "name": "Abel's iPhone", "platform": "ios", "key": "<b64 device public key>" }
   ```
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

## Relay API

Base URL: the relay's (`https://relay.equipe.im` once deployed; `EQUIPE_RELAY` overrides it).

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
| `pair_recv` | `slot`, `data` | a sealed pairing request |
| `presence` | `device`, `online` | a device connected or left |
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

`title` and `body` are what the lock screen shows — who needs what, never the command or
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
