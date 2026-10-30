# Judge access — design and the owner's decisions

**Status: designed and implemented in the repository (`equipe judge-relay`), tested only against a
local mock upstream. Not deployed, never run against the real API, no key or budget approved.**

## The problem

The rules require that judges can test the project **free of charge and without restriction
until 2026-12-23 12:00 PT**. The NVIDIA-on-Nebius part (`equipe review`, and the daemon's
automatic review) needs a Token Factory key. Options:

| Option | Judges need | Status |
|---|---|---|
| A. Bring your own key (judge's own Nebius account) | A Nebius account, a card on file, their own credits | **Not confirmed acceptable** under "free and without restriction". Needs the organizers' written confirmation before relying on it. |
| B. Owner's key handed to judges | Nothing | **Not recommended**: the key would be shared in the clear, with no cap the owner controls, and couldn't be taken back from one judge only. |
| C. **Judge relay** (below): the owner's key stays on a small server; judges get app tokens | Nothing but the token and two environment variables | Implemented, not deployed. Needs the owner's decisions listed at the end. |

The full squad (agents that write code) additionally needs Claude Code or Codex on the judge's
own plan; no option here changes that (see [judge-paths.md](judge-paths.md)).

## Design (option C)

```
 judge's machine                              relay host (owner's)                       Nebius
 ───────────────                              ────────────────────                       ──────
 NEBIUS_API_KEY=<app token>                   TLS proxy (e.g. Caddy, automatic HTTPS)
 EQUIPE_NEBIUS_BASE_URL=https://<host>/v1/      │
 equipe review  ── HTTPS ───────────────────▶ equipe judge-relay (127.0.0.1:8787)
                                                │  app token → SHA-256 → known name?    401 if not
                                                │  past EQUIPE_RELAY_UNTIL?             403
                                                │  GET /v1/models → only nvidia/…nemotron… ids
                                                │  POST /v1/chat/completions:
                                                │    model must be an NVIDIA Nemotron chat model   400 if not
                                                │    no streaming; max_tokens ≤ cap; body ≤ 256 KB
                                                │    per-token and global daily caps   429 + Retry-After
                                                │  Authorization: Bearer $NEBIUS_API_KEY ──────────▶ Token Factory
                                                │  log: token name, status, model, tokens,
                                                │       request id, latency — never key/token/body
```

- **Code**: `crates/cli/src/judge_relay.rs` (hidden subcommand `equipe judge-relay`). Uses the same
  model filter as the client (`nemotron::nemotron_models`), so it cannot be used for other models.
- **No client change**: judges run the normal `equipe review`; the relay speaks the same API.
- **The key** lives only in the relay process's environment (e.g. a systemd `EnvironmentFile` with
  mode 600). App tokens are stored only as SHA-256 hashes; a token is printed once when made.
- **Caps** (defaults, all settable): 40 requests and 150,000 tokens per app token per UTC day;
  400 requests per day in total; answers capped at 2,048 tokens. A request is counted *before*
  it is forwarded, so a burst can't overshoot the request cap. Tokens are counted from
  Token Factory's reported usage after each answer.
- **End date**: `EQUIPE_RELAY_UNTIL` (Unix seconds). Proposed: **2027-01-15 00:00 UTC**, three weeks
  past the 2026-12-23 12:00 PT judging end, so access never lapses early.
- **Settings**: `NEBIUS_API_KEY`, `EQUIPE_RELAY_TOKENS` (`name:sha256hex,…`), `EQUIPE_RELAY_UNTIL`,
  `EQUIPE_RELAY_REQUESTS_PER_DAY`, `EQUIPE_RELAY_TOKENS_PER_DAY`,
  `EQUIPE_RELAY_GLOBAL_REQUESTS_PER_DAY`, `EQUIPE_RELAY_MAX_TOKENS`, `EQUIPE_NEBIUS_BASE_URL`.

### Tested (mock upstream only)

`cargo test -p equipe judge_relay` — equipe's own client → relay → a mock Token Factory on localhost:
a review goes through with the relay's key (the app token never reaches upstream) and `max_tokens`
is capped; models are cut to NVIDIA Nemotron; an unknown token, another vendor's model, and a request
after the end date are refused without reaching upstream; the daily cap refuses the third request
and allows again the next day; settings never show the key.

### Known limits (not done)

- Counts are in memory: a restart starts the day's counts over (the global cap still bounds a day
  unless it restarts repeatedly). No persistence, no per-IP limit on refused requests,
  `GET /v1/models` isn't counted.
- Plain HTTP/1.1 with `content-length`, one request per connection: it must sit behind a TLS proxy.
- Not load-tested, not externally security-reviewed, never run against the real Token Factory.

## Running it (for the owner, once decided — not done)

```bash
equipe judge-relay token judges          # prints the app token (give it to judges) and its hash line
# On the relay host, in a root-only env file (chmod 600), e.g. /etc/equipe-relay.env:
#   NEBIUS_API_KEY=<the owner's Token Factory key>
#   EQUIPE_RELAY_TOKENS=judges:<sha256 hex from above>
#   EQUIPE_RELAY_UNTIL=1799971200        # 2027-01-15T00:00:00Z — check with: date -u -r 1799971200
equipe judge-relay --listen 127.0.0.1:8787
# TLS in front, e.g. Caddyfile:  relay.example.com { reverse_proxy 127.0.0.1:8787 }
```

Judges then run:

```bash
export NEBIUS_API_KEY=<app token> EQUIPE_NEBIUS_BASE_URL=https://relay.example.com/v1/
equipe review --models && equipe review
```

## Cost — an estimate, nothing measured

Per review, at most ≈14,000 input tokens (48,000-character diff + ticket + instructions) and
2,048 output tokens; typical changes are much smaller. Worst case per day with the default global
cap: 400 × (14,000 + 2,048) ≈ 6.4 M tokens. At **hypothetical** prices of $0.10 / 1 M input and
$0.40 / 1 M output tokens that is ≈ $0.88 per day, ≈ $50 for 55 days at the cap every day. Real
prices depend on the model and must be read from Token Factory's pricing page; real usage by judges
is likely far below the cap. These numbers are not a measurement.

**The relay's caps are the only cap in this design.** Token Factory onboarding needs a card and the
account can be charged when the balance goes negative; promotional credits are not a spending cap,
and no per-key dollar limit or automatic key expiry is assumed (none is documented as far as this
design relies on). The owner should check the billing documentation
(<https://docs.tokenfactory.nebius.com/other-capabilities/billing-new>) before deciding.

## Decisions only the owner can make

1. Which option (A, B, C, or several) — and, for A, whether to ask the organizers for written
   confirmation that bring-your-own-key is acceptable.
2. For C: whether to deploy at all; where (a small VM, possibly on Nebius AI Cloud, or elsewhere);
   the domain and TLS; who operates and monitors it until the end date.
3. The budget and the caps (per token, global, max tokens), and the end date.
4. Whether to create a dedicated Token Factory key for the relay (recommended, so it can be revoked
   alone), and when to revoke it after judging.
5. How judges receive the app token (e.g. the Devpost "testing instructions" field, if it is only
   visible to judges — to be confirmed), one shared token or one per judge.
6. Which model id to pin (`EQUIPE_NEBIUS_MODEL` on the judges' side is optional; the relay accepts
   any NVIDIA Nemotron chat model the account offers).
