# Telemetry: anonymous usage events, opt-in

equipe can send a few anonymous events about how a squad works — never what
it works on. Nothing is sent until the person says yes, and this page is the
whole list: if an event or a property isn't here, equipe doesn't send it. The
code is `crates/core/src/telemetry.rs`; its tests hold every event to the
allow-list below.

## Consent

The answer is one of three, kept in `~/.equipe/telemetry.json`:

| `consent` | What is sent |
|-----------|--------------|
| `unset`   | Nothing. This is how every install starts; the desktop app asks on its first run. |
| `on`      | The events below. |
| `off`     | Nothing. |

Turn it on or off — the daemon reads the file before every send, so it holds
at once:

```bash
equipe telemetry on
equipe telemetry off
equipe telemetry status
```

A screen does the same over the daemon's protocol (`docs/remote-protocol.md`
framing): `telemetry` → `{ consent, sending, held_by? }`, and
`telemetry_set { on }` → the same.

Whatever the answer, nothing is sent when the daemon's environment has
`EQUIPE_TELEMETRY=off` (or `0`, `false`, `no`), or `CI` set (anything but
`false` or `0`). `status` says which one holds it back.

Turning it off forgets the install's id and every squad's: a later `on`
starts as a new, unrelated install.

## Who sends

The daemon, and only the daemon: the desktop app, the terminal and the phone
all create tickets and approve work through it. A terminal's own runtime
without a daemon sends nothing.

## Anonymous ids

- **Install id** (`distinct_id`): a random UUID (v4, from `/dev/urandom`),
  made when the person says yes. It is not derived from the machine, the
  user, or anything else — no hardware id, no hash of one.
- **Squad id** (`squad`): a random UUID made the first time a squad does
  something after the yes. The map from a squad's name to its id stays in
  `telemetry.json`; the name never leaves the machine.

Every event is sent with `$process_person_profile: false` (PostHog keeps no
person profile) and `$geoip_disable: true` (no place looked up from the IP).

## Events

Every event carries these properties:

| Property      | Example   | |
|---------------|-----------|-|
| `app_version` | `0.86.4`  | equipe's version |
| `os`          | `macos`   | `macos` or `linux` |
| `$lib`        | `equipe-daemon` | |
| `$process_person_profile` | `false` | |
| `$geoip_disable` | `true` | |

And each its own:

| Event            | When | Properties |
|------------------|------|------------|
| `first_run`      | Once per install, the first time the daemon runs after the yes (within 10 minutes of it, or at the first event). | — |
| `squad_created`  | The first time a squad does something after the yes — a new squad, or one that existed before the yes. | `squad`, `team_size` (members running then) |
| `ticket_created` | A ticket is put on the board (proposals too; not design QA tickets equipe opens itself). | `squad`, `kind` (`code` · `task` · `action`), `by` (`person` · `pm` · `member`) |
| `work_handed_in` | A member hands in a ticket for review. | `squad`, `kind` |
| `work_accepted`  | Handed-in work is accepted. | `squad`, `kind`, `by` (`person`: the person accepted, on any screen; `pm`: the PM accepted it, where the person lets it) |
| `daily_active`   | Once per squad per UTC day it did something, sent once the day is over (timestamped at its last second). | `squad`, `day` (`2026-11-01`), `person_accepts` (the person's accepts that day, as a range: `0` · `1` · `2-4` · `5-9` · `10+`), `engines` (`claude` · `codex` · `claude+codex`) |

## Never sent

- Conversations: no message, prompt, reply, summary or note.
- Tickets' contents: no title, body, comment, branch or image.
- Files and paths: no file name, folder, repository or URL.
- Names: no squad name, member name, project name, or the person's name.
- Contact: no email, phone, account or sign-in.
- The machine: no hostname, serial, MAC address, or a hash of any of them.

## Where it goes

PostHog US, project 700951 — the same project as equipe.im's analytics, so
the site → download → first approval funnel is in one place. Events go to
`https://us.i.posthog.com/batch/` with the project's public key (`phc_…`,
write-only by design; it is in the site's pages too).

Delivery is best effort: events wait in a small in-memory queue (256; more
are dropped), go in batches of up to 50, a failed request is tried once more
30 seconds later and then dropped. Nothing is written to disk to send later
but the day's counts for `daily_active`.

PostHog sees the request's IP address, as any server does. GeoIP is off for
these events; whether the project also discards the IP is a project setting
(Settings → Project → "Discard client IP data"), shared with the site.

## The North Star: weekly approved squads

Squads where the person accepted at least one piece of work in the week:

```sql
-- PostHog SQL (HogQL), the last 7 days
SELECT count(DISTINCT properties.squad)
FROM events
WHERE event = 'work_accepted'
  AND properties.by = 'person'
  AND timestamp >= now() - INTERVAL 7 DAY
```

Or as an insight: Trends → `work_accepted` where `by = person`, aggregated
by unique `squad` (a property count), weekly. A PM's accept doesn't count:
the North Star is the person approving what the squad made.

## Changing this contract

Adding an event or a property means changing this page in the same change,
and the allow-list in `crates/core/src/telemetry.rs`'s tests with it.
