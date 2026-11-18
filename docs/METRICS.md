# Metrics

A maker connects the services their product reports to — PostHog, GitHub, App
Store Connect — and the squad and the person read the numbers inside equipe:
the desktop's Metrics screen in full, the phone as a summary card, the PM
through its `metrics` tool for the morning report and experiment calls.

There is no equipe server in it. **The daemon on the person's Mac calls each
service's API itself**, keeps the keys in that Mac's Keychain, and keeps what
it hears as daily snapshots under `~/.equipe/metrics/`. A screen on another Mac
or the phone gets numbers from that daemon, never keys.

Code: `crates/protocol/src/metrics.rs` (the wire types), `crates/core/src/metrics/`
(storage, fetches, periods, the tool), the requests in `crates/core/src/gateway.rs`.
Screens: `design/specs/metrics/` (T-311).

```mermaid
flowchart LR
  S[Desktop / phone] -- metrics.* JSON-RPC --> D[daemon]
  M[member: metrics tool] --> D
  D -- keys --> K[(Keychain: equipe's one item)]
  D -- API with key --> P[PostHog / GitHub / App Store]
  D --> F[(~/.equipe/metrics/)]
```

## Rules

- A key is in the Keychain and nowhere else: not `connections.json`, not a
  snapshot, not the daemon's log, not an answer, not a member's tool result.
  `metrics.connect` is the only message that carries one, inward.
- The keys live in equipe's one Keychain item (service `equipe`, account
  `secrets`, `crate::keychain`), beside logins' passwords, under
  `equipe.metrics:<connection id>`; items an older equipe kept per connection
  are moved into it when first read. The daemon reads the item once, at
  start, and keeps it in memory: one Keychain dialog at most, never one per
  connection, never one per fetch.
- The item trusts the `equipe` that made it by its code signature. An ad-hoc
  build (cdhash only) is a stranger to the next one — `errSecAuthFailed` or a
  dialog, error code `keychain` (T-356). Release builds are signed with the
  Developer ID (docs/RELEASING.md), so "Always Allow" lasts across updates.
  After a no, background fetches don't ask again for a day.
- An adapter's error message has every key of its connection cut out
  (`•••`) before it is kept, answered or logged — in case a service echoes it.
- Members only read. Connecting, keys and disconnecting are the person's, on
  their screen.
- Files under `~/.equipe/metrics/` are the person's only (folder 0700, files 0600).

## Sources

A **source kind** is a service. A **connection** is one source set up for a
squad, for one of its projects (`project`) or the squad as a whole (none).
A squad may have several connections of a kind (two GitHub repositories).

`metrics.sources` lists the kinds there are (`SourceSpec`): their fields and
the metrics they report. A screen builds its connect form from it — it doesn't
hard-code fields. A kind is offered only once its adapter exists.

| kind | status | fields (`*` secret, kept in the Keychain) | metrics |
|---|---|---|---|
| `fake` | here (tests, screens; offered when `EQUIPE_METRICS_FAKE` is set) | `token`*, `seed`, `fail` | `visits`, `stars`, `languages` |
| `posthog` | here (`adapters/posthog.rs`; "Set it up for me" below) | `api_key`* (Query: Read), `project_id`, `host` (default `https://us.posthog.com`), `filter`, `language_property`, `click_events`, `click_property`, `north_star_event`, `north_star_property` (`distinct_id`: people), `north_star_where` (`*`: every event) | `visits`, `languages`, `referrers`, `utm_sources`, `clicks`, `north_star` |
| `github` | here (`adapters/github.rs`) | `token`* (Administration: read), `repo` (`owner/name`) | `views`, `visitors`, `clones`, `stars`, `referrers` |
| `app_store` | here (`adapters/app_store.rs`) | `issuer_id`, `key_id`, `private_key`* (the .p8's text), `vendor_number`, `app_id` | `downloads` |

Each adapter's `spec()` is the truth for its fields and metrics; the hints
there say where in the service each one is found.

- **PostHog** asks HogQL (`/api/projects/<id>/query/`), days cut by this Mac's
  time zone. `visits` is page loads, not people, on a site that gives each
  load a new person ID. `filter` (`internal!=true, environment=production`) is
  the person's to set, never guessed. A breakdown keeps a day's 30 biggest
  labels and adds the rest up as `(other)`. Without `click_events` no clicks
  are asked.
- **PostHog's `north_star`** is docs/TELEMETRY.md's North Star: the distinct
  `squad`s of `work_accepted` events with `by = person` in the 7 days up to
  each day (`rolling`). `north_star_event` (default `work_accepted`),
  `north_star_property` (default `squad`) and `north_star_where` (default
  `by=person`, written like `filter`) change what is counted; `filter` isn't
  applied to it — those are the site's page loads' properties. One HogQL query
  a fetch counts every day (each day and squad seen stands for the 7 days
  whose window it is in), asking 91 days at most (12 windows of 7 and the one
  before), 14 later. A day before the first squad seen is left out (unknown);
  after it, a day with none is 0. With no event at all and none named (a
  maker's product doesn't send equipe's `work_accepted`), it isn't reported
  and the screen shows "no definition"; a named event with none is 0.
- **GitHub** traffic (`views`, `visitors`, `clones`) is the API's last 14
  days (`keeps_days` 14); `stars` is a day's running total from the
  stargazers' dates when they fit in 10 pages (1,000), else today's count
  only. `referrers` is GitHub's 14-day window, so it is a `total` split
  by label: a period shows its last day's split, not a sum of overlapping
  windows.
- **App Store Connect** reads the daily Sales report (`/v1/salesReports`,
  a JWT signed with the .p8 on this Mac): `downloads` is first-time
  downloads of `app_id`. Today is never asked; a day Apple has no report for
  yet is left out, and only a day it says sold nothing is 0. Apple keeps
  daily reports about a year (`keeps_days` 365), so a first fetch asks a
  year at most.

`fake` makes up numbers that look like a small site's, the same for the same
`seed` and day, and — like GitHub — remembers 14 days. `fail` (`auth`,
`rate_limited`, `network`) makes every fetch fail, its message quoting the
token so the redaction can be tested.

### PostHog: "Set it up for me"

Making a PostHog key by hand is five steps on someone else's site (Settings →
Personal API keys → create → tick scopes → copy, then paste it with the project
ID). The person signs in to PostHog once instead, and equipe does the rest.
Metrics' "Connect PostHog" offers **Set it up for me** first and **Paste a
key** beside it. Code: `crates/core/src/metrics/setup.rs`, the browser host's
`posthog` op (`crates/core/assets/browser-host.cjs`), `browser::Hub::ask`, the
screen in `crates/desktop/src/views/metrics.rs` (logic: `Setup` in
`crates/desktop/src/metrics.rs`).

```mermaid
sequenceDiagram
  participant S as Screen
  participant D as daemon
  participant B as equipe's browser (host)
  participant P as PostHog Cloud
  S->>D: metrics.posthog_look
  D->>B: posthog look (equipe's own tab, person's session)
  B->>P: GET /api/users/@me/ (US, EU)
  B-->>D: signed in where, organization, projects
  Note over S: signed out → PostHog's sign-in opens in equipe's browser;<br/>the person types password / code; looked at again every 4 s
  Note over S: several projects → a choice card
  Note over S: the confirm card: name, scope, project — the person's yes
  S->>D: metrics.posthog_setup {host, project_id}
  D->>B: posthog key {label, scopes, teams}
  B->>P: POST /api/personal_api_keys/ (X-CSRFToken)
  B-->>D: the key's value — to the daemon alone
  D->>P: HogQL top events (30 days), with the new key
  D->>D: Keychain ← key; connection: project, host, North Star
```

1. **Look.** `metrics.posthog_look` asks the browser host, in a tab of
   equipe's own (lane `equipe:posthog`: behind the person's tabs, no member's
   tools on it), PostHog's `GET /api/users/@me/` on `us.posthog.com` and
   `eu.posthog.com`. Signed in at one: its organization and projects, and the
   region — the address the session is at. Not signed in: the screen opens
   PostHog's sign-in in equipe's browser through the usual "Sign in to a site"
   (`Command::BrowserSignIn`); the person types their password and any code
   themselves, and the screen looks again every 4 s for half an hour.
2. **Pick.** One project: on. More: a choice card (project · organization ·
   region).
3. **The person's yes.** A card says exactly what will be made — name
   `equipe metrics (read-only)`, access `query:read` only, the project — and
   that the key goes to this Mac's Keychain and where to delete it. Only its
   **Create the key** sends `metrics.posthog_setup`; members have no tool or
   request that reaches it.
4. **Make the key.** The host calls `POST /api/personal_api_keys/` — the
   request PostHog's own settings page makes, with its CSRF token from the
   page's cookie — with `scopes: ["query:read"]`, `scoped_teams: [project]`.
   Listing projects uses the person's session, so the key needs no
   `project:read`. The answer (`{"type":"answer"}`) goes from the host's
   output straight to the asking request (`Hub::ask`), never through the
   runtime's events or a log.
5. **Keep and connect.** The key becomes the connection's `api_key` (the
   Keychain, as a pasted key), with `project_id` and `host` set. The new
   key asks the project's top events of 30 days: no `$pageview` — the visit
   cards make way for one card that says the project sends no page views (not
   "unreachable"); no `work_accepted` — the North Star counts the people
   (`north_star_property: distinct_id`) who sent the project's most sent event
   of its own (`north_star_where: *`), shown on the done card with "Change
   definition". The first fetch follows.

**Security.** The key's value is in the host's answer, the daemon's memory and
the Keychain — never on a page a member can read, a screen, an event, a log, a
URL or a screenshot (`Minted`'s `Debug` hides it; the setup's answer has none).
Scope is the least the screen needs, on one project. The done card names the
key and links PostHog's key page to delete it.

**When it can't be done for the person.** PostHog makes a key only for a
sign-in from the last two hours (`403 sensitive_action_required_reauth`): the
card says so, and **Sign in again** signs out of PostHog in equipe's browser
(PostHog's own `POST /logout`, from equipe's tab) and opens the sign-in there
for the person; once they are in, the same yes card comes back. Signed out meanwhile: back to signing in.
Anything else (a limit on keys, no answer, members using a Chrome each, a
self-hosted PostHog): the key page opens in equipe's browser with the steps,
beside the paste boxes. What stays manual: a self-hosted PostHog (only US and
EU Cloud are looked at), the password and any code (always the person's),
and deleting an older equipe key when one is made again.

**Suggested after a ticket.** When a member saves a PostHog key in the squad's
keys (`secret_save`, its service or name saying PostHog — on a ticket that wired
PostHog into the product, say) and Metrics has no PostHog yet, Metrics shows
"Show these numbers in Metrics?" with **Set it up for me**, and Connections'
PostHog row a **Show in Metrics** button: one tap starts the same flow.

### Field

```json
{ "key": "api_key", "label": "Personal API key", "secret": true, "required": true, "hint": "PostHog → Settings → Personal API keys" }
```

`label` is English; screens translate by `key`.

## Metrics

Each source reports metrics by **name** (unique within the source), with a
**unit** and a **shape**:

| shape | a day is | a period is |
|---|---|---|
| `count` | that day's count | the sum of its days |
| `total` | a running total that day (stars), maybe split by label | its last known day (and that day's split) |
| `breakdown` | that day's count, split by label (`breakdown`) | each label summed, biggest first |
| `rolling` | the distinct count of the 7 days up to that day (squads) | its last known day; `previous` is the day 7 days before it, and `series` 12 days 7 apart ending there (`previous_series` empty), whatever `days` and `bucket` |

Units: `page_loads`, `clicks`, `views`, `clones`, `stars`, `downloads`, `squads`. Screens
word them ("page loads"). PostHog gives each page load a new ID, so `visits` is
page loads, not people.

A day is the daemon machine's **local calendar day**, `YYYY-MM-DD`. A day a
service doesn't know yet (App Store reports a day late) is left out, never 0:
it shows as a gap, and `as_of` says the latest day known.

## Periods

A query is a period of `days` (1–366, default 7) ending `to` (default today),
beside the **previous period**: the same number of days just before it.
`bucket` cuts its series: `day`, or `week` (Monday to Sunday; the first and
last buckets are cut at the period's ends, and each is named by its first day).
The screens' "7 days / 30 days" are `days: 7` and `days: 30`.

`change` is `value − previous`; `change_pct` is that over `previous`, in percent,
rounded to 0.1, absent when there is no previous value or it is 0.

## Snapshots

The daemon fetches each connection

- right after `metrics.connect` (asking up to 400 days back on the first fetch;
  an adapter may ask less — PostHog 92 days of page loads and clicks, its
  `north_star` 91);
- once a day from **07:00 local** (asking the last 14 days);
- after a failed fetch, again when the service said (`retry_at`, at any hour)
  or an hour later (from 07:00);
- on `metrics.refresh`.

### A busy service

A service that answers "not now" is waited for, never reported as the
internet being down. PostHog answers a query it has no room for with
`503 clickhouse_at_capacity` ("Queries are a little too busy right now…")
and a `Retry-After` of 5–20 s; any service may answer 429 (over a limit), or
502/504 from a gateway.

- Within a fetch, a request answered 429/502/503/504 is asked again up to
  twice, after `Retry-After` (else 3 s, then 9 s), while the fetch's 40 s of
  patience for all its requests lasts (`adapters::Patience`).
- Still busy, the fetch fails `rate_limited` with the `status` and the
  service's own words (`said`, keys cut out). A 503 (`FetchError::busy`) is
  tried again 5 minutes later, then twice as long each time it is still
  busy, an hour at most — never sooner than the service said. The daily loop
  wakes for it (it looks every 10 minutes, or sooner when a retry is due).
- PostHog's queries are kept light: one at a time, 1.5 s apart, 92 days at
  most on a first fetch (not 400 — what tripped PostHog's capacity on a fresh
  connection), the time range bounded on both sides; a project that sends no
  page loads isn't asked for their referrers and UTM sources.

The screen says it in the service's words — "PostHog is too busy right now ·
PostHog said: “Queries are a little too busy right now…” Trying again
automatically at 10:42." — and "What you see is from …" only when earlier
numbers are shown.

One fetch per connection at a time; a second ask waits for the first's numbers.

What a fetch brings is kept as that day's snapshot:

```
~/.equipe/metrics/
  connections.json                       every connection (no keys)
  <squad>/<connection id>/<YYYY-MM-DD>.json   one per day fetched; a later fetch that day replaces it
```

```json
{
  "schema": 1,
  "connection": "m1",
  "kind": "github",
  "fetched_at": 1793520000,
  "points": [
    { "metric": "views", "day": "2026-11-01", "value": 312 },
    { "metric": "languages", "day": "2026-11-01", "value": 120, "breakdown": { "English": 70, "日本語": 50 } }
  ]
}
```

Reading, the snapshots are taken in day order and a later snapshot's value of a
day wins. So a day GitHub has forgotten (it keeps traffic for 14 days) stays as
the last snapshot that had it. Disconnecting keeps a connection's snapshots
unless asked to `forget`.

`schema` is bumped when a snapshot's meaning changes; the daemon reads only
schemas up to its own. Only points of the source's metrics, of real days, none
after today, are kept.

## JSON-RPC

Requests over the daemon's socket (`docs/ARCHITECTURE.md`, `protocol::rpc`),
answered on their own task. None needs `hello` first; each names its `squad`.

### `metrics.sources`

`{ "squad": "acme", "project"?: "equipe-site" }` → `Sources`

```json
{
  "kinds": [ { "kind": "fake", "label": "Fake", "fields": [...], "metrics": [...], "keeps_days": 14 } ],
  "connections": [ {
    "id": "m1", "squad": "acme", "project": "equipe-site", "kind": "fake",
    "settings": { "seed": "3" }, "keys": ["token"], "connected_at": 1793400000,
    "last": { "at": 1793520000, "error": { "code": "auth", "message": "token ••• was refused" } },
    "last_ok": 1793433600
  } ]
}
```

`keys` names the secret fields kept — never their values. `last` is the latest
fetch (with `error` when it failed); `last_ok` when one last worked. A screen
shows "last value, dimmed, with its time" from `last_ok` when `last.error` is set.

### `metrics.connect`

`{ "squad", "project"?, "kind", "settings": { every field, secret ones included } }` → `Connection`

With `"id": "m1"` instead of `kind`: new settings or keys for that connection; a
secret field left out keeps its key, a setting left out keeps its value; an
optional (not required, not secret) setting sent as `""` is cleared. The
answer comes at once; the first fetch follows on its own — `metrics.sources`
or `metrics.refresh` shows when it has.

### `metrics.posthog_look`, `metrics.posthog_setup`

`{ "squad", "sign_out"?: "https://us.posthog.com" }` → `PosthogLook { browser, accounts: [{ host, region, organization, projects: [{ id, name }], current? }] }` (`sign_out`: the person's "Sign in again" — PostHog's own sign-out on that host first).

`{ "squad", "project"?, "host", "project_id", "id"? }` → `PosthogSetup { outcome: connected | sign_in | sign_in_again | manual, connection?, key_label, scopes, keys_url, events, page_views, north_star_event?, why? }`
— the person's yes only; never carries the key. See "PostHog: Set it up for me".

### `metrics.disconnect`

`{ "squad", "id", "forget"?: false }` → `{ "id" }`. Its keys leave the Keychain.

### `metrics.refresh`

`{ "squad", "project"?, "id"? }` → `Sources`, once every fetch asked for is done
(up to 90 s each). A failed fetch isn't an error of the request: it is in that
connection's `last.error`.

### `metrics.query`

`{ "squad", "project"?, "days"?: 7, "to"?: "YYYY-MM-DD", "bucket"?: "day" | "week", "connections"?: ["m1"], "metrics"?: ["visits"] }` → `QueryResult`

```json
{
  "from": "2026-10-27", "to": "2026-11-02", "previous_from": "2026-10-20", "previous_to": "2026-10-26",
  "days": 7, "bucket": "day",
  "values": [ {
    "connection": "m1", "kind": "posthog", "metric": "visits", "label": "Visits", "unit": "page_loads", "shape": "count",
    "value": 1284, "previous": 1187, "change": 97, "change_pct": 8.2,
    "as_of": "2026-11-02", "days_known": 7,
    "series": [ { "day": "2026-10-27", "value": 160 }, { "day": "2026-10-28", "value": null } ],
    "previous_series": [ ... ],
    "breakdown": [ { "label": "English", "value": 742, "previous": 700 } ]
  } ],
  "connections": [ ... ]
}
```

`value` is `null` when no day of the period has one. `breakdown` only for
`breakdown` metrics. A query reads the snapshots only — it never calls a
service nor reads a key.

### `metrics.summary`

`{ "squad", "project"? }` → `Summary`: the phone's card — the last 7 days
beside the 7 before, the first four metrics that aren't breakdowns, in the
order of the connections and their sources' metrics, but `north_star` first
(its tile: the last day's squads beside 7 days before).

```json
{
  "squad": "acme", "project": "equipe-site", "from": "2026-10-27", "to": "2026-11-02",
  "tiles": [ { "connection": "m1", "kind": "posthog", "metric": "visits", "label": "Visits", "unit": "page_loads",
               "value": 1284, "previous": 1187, "change_pct": 8.2, "as_of": "2026-11-02",
               "error": { "code": "auth", "message": "..." } } ],
  "updated_at": 1793520000
}
```

A tile's `error` is its connection's latest failed fetch: the value is the last
one kept. The phone asks it through the relay (`docs/remote-protocol.md`, "The
project's numbers"): `metrics.summary` is the only `metrics.*` a device may
ask; keys never go to the phone — the person reconnects on the Mac.

## Errors

A request that can't be answered is a JSON-RPC error:

| code | when |
|---|---|
| -32602 | params missing or wrong (`days` outside 1–366, a bad `to`, no squad) |
| -32601 | no such `metrics.*` method |
| -32010 | that kind can't be connected yet (no adapter) |
| -32011 | a required field is missing, or a field the kind doesn't have |
| -32012 | no such connection in that squad |
| -32013 | the Keychain refused to keep or remove the keys |
| -32014 | `~/.equipe/metrics/` couldn't be written |

A **fetch** that fails isn't a request error; it is kept on the connection as
`last.error`, `{ code, message, retry_at?, status?, said? }` — `status` the
HTTP status the service answered (absent when it couldn't be reached),
`said` its own words, clipped, every key cut out:

| code | the screen says |
|---|---|
| `auth` | the key was refused or expired — put in a new one (`metrics.connect` with `id`) |
| `rate_limited` | the service said not now — over a limit (429), or busy (503: "too busy right now") — tried again at `retry_at` |
| `network` | the service couldn't be reached, or didn't answer in 90 s — never a status it answered |
| `bad_response` | the service answered something equipe couldn't read |
| `keychain` | the key couldn't be read from the Keychain |
| `other` | anything else |

## The members' tool

`metrics` (every member; read only): `{ project?, days?, to?, metric?, by_week? }`.
It answers in text — each connection with its latest fetch (or its failure),
each metric's value beside the previous period with the change, the days known,
a breakdown's top labels and the series. It reads the snapshots only; nothing
in it comes from the Keychain. With nothing connected it says the person
connects services in equipe's Metrics screen.

## Adding an adapter

One file per service: **`crates/core/src/metrics/adapters/<kind>.rs`**
(`posthog.rs`, `github.rs`, `app_store.rs`), implementing `adapters::Adapter`:

```rust
pub trait Adapter: Send + Sync {
    fn spec(&self) -> SourceSpec;                       // fields, metrics, keeps_days
    fn fetch<'a>(&'a self, fetch: Fetch<'a>) -> Fetching<'a>; // → Vec<Point> or FetchError
}
```

`Fetch` gives the settings, the keys (from the Keychain), today and how many
days back to ask for, and the daemon's HTTP client. Then, in
`adapters/mod.rs` only, its `pub mod` and its arm in `adapter()`. An adapter
sends its keys to its own service and nowhere else; it returns days it knows
(not zeros for days it doesn't) and maps the service's refusals to
`auth` / `rate_limited` (with `retry_at` from `Retry-After`) / `network` /
`bad_response` (`adapters::refused` does it for an HTTP answer, and
`adapters::Patience` asks a busy service again). Everything else — keys, snapshots, periods, comparisons,
redaction, the schedule — is `metrics`' own, so adapters on separate tickets
don't touch the same files beyond those two lines.

Tests: the fake adapter (`adapters/fake.rs`) drives connect → refresh → query
in `crates/core/src/metrics/tests.rs` and over the socket in
`crates/core/tests/gateway.rs`; both check that the key is in no file, answer
or tool result.
