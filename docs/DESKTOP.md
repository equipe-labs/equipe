# Equipe desktop

All-Rust desktop app: `crates/core` (board + agent logs in SQLite) and
`crates/desktop` (UI on [GPUI Kit](https://github.com/longbridge/gpui-kit) 0.7.1,
which pins its GPUI snapshot, `gpui-pre` 0.3.8).

```bash
cargo run --release -p equipe-desktop
```

Why a Rust core and GPUI: [research/desktop-stack](./research/desktop-stack/README.md).

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
