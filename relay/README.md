# equipe relay

The mailman between `equipe daemon` and paired phones: a Cloudflare Worker with one Durable
Object per daemon. Daemon and devices both connect out to it over WebSockets; it passes
sealed frames between them and sends pushes through APNs and FCM. It never holds a key that
opens a frame. The protocol is in [`docs/remote-protocol.md`](../docs/remote-protocol.md).

- `src/index.ts` — the Worker: `POST /v1/daemons` (rate-limited per IP) and routing to the daemon's object
- `src/daemon.ts` — the per-daemon Durable Object: sockets (hibernatable), devices, pairing slots
- `src/frames.ts` — frame types and validation
- `src/budget.ts` — how much a device may send a minute (a phone's files go a piece at a time)
- `src/push.ts` — APNs (ES256 provider token) and FCM HTTP v1 (service-account OAuth)

## Test

```sh
npm ci
npm test            # vitest inside workerd, no Cloudflare account needed
npx tsc --noEmit
```

## Run locally

```sh
npx wrangler dev    # http://localhost:8787; point the daemon at it with EQUIPE_RELAY
```

Push secrets for local runs go in `.dev.vars` (ignored by git).

## Deploy

```sh
npx wrangler login
npx wrangler deploy
npx wrangler secret put APNS_KEY             # contents of the AuthKey_XXXX.p8 file
npx wrangler secret put APNS_KEY_ID
npx wrangler secret put APNS_TEAM_ID
npx wrangler secret put APNS_TOPIC           # the app's bundle id
npx wrangler secret put FCM_SERVICE_ACCOUNT  # the service account JSON
```

Each push platform stays off, answering `push_result` `ok: false`, until its secrets are set.
