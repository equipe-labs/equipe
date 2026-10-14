# Equipe desktop

All-Rust desktop app: `crates/desktop` (UI on [GPUI Kit](https://github.com/longbridge/gpui-kit) 0.7.1,
which pins its GPUI snapshot, `gpui-pre` 0.3.8). It is a window onto the daemon —
a third screen beside the terminal console and the phone — not a second engine.

```bash
cargo run --release -p equipe-desktop
```

Why a Rust core and GPUI: [research/desktop-stack](./research/desktop-stack/README.md).
Screen design: the squad's `outputs/T-54 데스크톱 화면 설계/spec.md`.

## How it is built: a daemon client

```
 equipe-desktop                                   equipe daemon
 ┌───────────────────────┐   ~/.equipe/equipe.sock   ┌───────────┐
 │ views/ ← state::Model │ ◄───── JSON-RPC lines ──► │ members,  │
 │            ▲          │                           │ board,    │
 │  daemon::Link (thread)│                           │ sessions  │
 └───────────────────────┘                           └───────────┘
```

- **Connection.** `daemon::Target::from_env` picks the socket (`$EQUIPE_HOME`, else
  `~/.equipe`, `equipe.sock`) and the squad (`$EQUIPE_SQUAD`, else the one opened
  last, else the first). `daemon::spawn` runs a `Link` on a thread of its own
  (small tokio runtime) that connects, follows, and when the daemon goes away waits
  and retries (250 ms doubling to 5 s) — so a daemon turned back on is picked up
  without a click. It reports only *changes*: a daemon that stays down is one
  `Down`, not one per retry.
- **Methods used** (the same door the terminal console uses, `equipe_protocol::rpc`):
  `hello {version, squad}` → `Hello {version, snapshot}` once per connection (the
  snapshot replaces whatever the window held), then `event` notifications
  (`Event`: `WorkStarted/WorkEnded`, `ToolUse`, `TextDelta`, `TurnEnded`,
  `ApprovalRequested/Resolved`, `TicketChanged`, `SessionStarted/Closed`, …).
  `command` (notification) is wired (`Link::send`) but no screen sends yet.
- **State.** `state::Model` folds the snapshot and events into the member list and
  the connection (`Connecting` / `Connected` / `Down`). A member "needs you" when it
  has a permission request open or owns a ticket put down for the person. Pure
  logic, unit-tested without a window. The squad list is read from the folder names
  under `<home>/squads` at start and on every attach.
- **Screens.** `src/views/{sidebar,chat,board}.rs`; `chat` and `board` are
  placeholders that later tickets fill in, each in its own file. `main.rs` holds the
  window, the daemon pump and the connected / banner / daemon-off bodies.
- **Drawing.** Updates that land within 100 ms are applied together; a frame is
  requested only if what is on screen differs (`Model::apply` returns whether it
  does). `EQUIPE_LOG_RENDER=1` prints a line per frame with the memory footprint.
  The daemon-off screen says to run `equipe`; the app does not start the daemon.

The old spike (mock agents into SQLite) is kept as `examples/mock_agents.rs`
(`cargo run --release -p equipe-desktop --example mock_agents`); the knobs below
are for it.

Spike knobs (env): `EQUIPE_AGENTS` (3), `EQUIPE_RATE` lines/s per mock agent
(200, `0` = idle), `EQUIPE_TICK_MS` UI poll interval (33), `EQUIPE_W`/`EQUIPE_H`.

## Rules that keep it light

1. **Logs never live in UI memory.** Agents append to SQLite in batched
   transactions (50 ms); each pane is a `uniform_list` that reads only the rows it
   is about to draw, by `(agent, seq)`.
2. **Draw nothing when nothing changed.** On macOS, any process that keeps
   presenting frames holds a ~105 MB GPU working set (see below). It is released
   ~1 s after the last frame. So no clocks, spinners or live readouts that repaint
   on their own; status text rides along with frames that happen anyway.
3. **Coalesce repaints.** The UI polls agent counters at a fixed tick and repaints
   only if something moved. Line rate does not change CPU, the tick does.

## Spike measurements

Apple Silicon, macOS 26.2, release build, 1200×760 window, `phys_footprint`
(what Activity Monitor shows). Single runs on one machine — orders of magnitude,
not benchmarks. 2026-10-11.

| | Result |
|---|---|
| Binary (stripped, thin LTO) | 12 MB (hello world: 9.9 MB, 4.4 MB gzipped) |
| Idle, 3 agent panes, board | **33 MB**, 0.2% CPU |
| Idle, daemon client (this app, sidebar + empty body, no events; 2026-10-14) | **34–38 MB**, 0.1% CPU — no frames between events |
| 3 agents streaming, tick 33 / 66 / 100 ms | 152 MB, 8.7% / 5.5% / 2.7% CPU |
| 3 agents × 3000 lines/s for 90 s (720k lines, 59 MB DB) | 153 MB, flat |
| Startup peak | ~141 MB, transient |
| First build / incremental dev rebuild / incremental release | ~75 s / 0.6 s / 15 s |

### The ~105 MB while drawing is macOS, not GPUI

| Probe (600×400 window) | Never redraws | Redraws every 1 s |
|---|---|---|
| Plain GPUI | 26 MB | 133 MB |
| GPUI Kit | 30 MB | 137 MB |
| Pure Swift + `CAMetalLayer` | 24 MB | 134 MB |
| Swift, Metal → IOSurface as `CALayer.contents` | 24 MB | 134 MB |
| Swift AppKit `draw(_:)`, no Metal | 16 MB | 129 MB |

The extra memory is reported as "Owned physical footprint (unmapped) (graphics)".
It does not scale with window size or window count. GPUI's own Metal allocations
(`device.current_allocated_size()`) stay at ~7 MB. Ghostty shows the same ~108 MB.
Redrawing every 2 s or more lets it drop between frames. So any UI stack that
draws in-process pays this while active. What we control is the idle floor and
how long we stay "active".
