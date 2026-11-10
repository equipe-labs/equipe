# T-582 long-conversation scroll profile

2026-11-09, macOS, release + snapshot, 1200×750 light, animation off
(`FAKE_WORKING=0`, `FAKE_ASKS=0`), with the hidden manual draw included in
`EQUIPE_PERF=2`. Each fixture settled for 3 seconds, then ran 240 wheel
events toward older and 240 toward newer entries.

## Deterministic counters

| State | fixture | direction | root render | conversation render | derived rebuild | chapter build |
|---|---:|---|---:|---:|---:|---:|
| before | 50/500/2,000 | each direction | 240 | 240 | 240 | 720 |
| after | 50/500/2,000 | each direction | 240 | 240 | 0 | 0 |

The conversation/root render ratio per scroll is 1.000 in every run.
Derived state and chapters were built once at the initial revision, and their
counters stayed unchanged through both scroll runs.

## CPU and memory

Quiet re-measurement, both directions combined (480 frames):

| fixture | CPU/frame mean | CPU p95 | CPU max | settled memory | memory after round trip |
|---:|---:|---:|---:|---:|---:|
| 50 | 5.726 ms | 8.347 ms | 13.947 ms | 78.4 MB | 64.9 MB |
| 2,000 | 3.499 ms | 7.565 ms | 11.363 ms | 69.6 MB | 68.5 MB |

The 2,000-entry fixture is within the budget of p95 8 ms and max 16.7 ms, and
its mean CPU/frame did not rise over the 50-entry fixture. The physical
footprint difference is within +30 MB and did not keep growing after the
round trip.

In the first measurement the 2,000-entry older-direction p95 dropped from
10.159 ms before to 8.500 ms after, but the machine's load average was 14, so
it was not used for the verdict. The table above is the re-measurement at load
average 3.23.

Raw data is in `baseline/app-{50,500,2000}.log`, `after/app-{50,500,2000}.log`
and `quiet/app-{50,2000}.log`.

After rebasing on the latest `origin/main`, the 2,000-entry fixture was checked
again under the same release and hidden-draw conditions. Over 480 frames,
CPU/frame was mean 5.612 ms, p95 7.151 ms, max 9.461 ms. In each 240-wheel run
the conversation rendered exactly 240 times, derived rebuild and chapter build
ran 0 times, and memory went from 68.0 MB settled to 64.0 MB after the round
trip. Raw data is in `rebased-app-2000.log`.
