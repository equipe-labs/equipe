# demo/ — the recording pipeline

Demos are day-1 infrastructure here: **every phase ships one gif**, so recording
has to be free. We script terminal GIFs with [VHS](https://github.com/charmbracelet/vhs)
so they're deterministic and regenerate from a text file — no hand-recording.

## Regenerate the hero GIF

```bash
brew install vhs      # one-time (pulls ttyd + ffmpeg)
pnpm demo             # builds, then renders demo/equipe.gif from demo/equipe.tape
```

`demo/equipe.gif` is embedded at the top of the root `README.md`.

## How it works

- `equipe.tape` is a VHS script (typed commands + timing). `pnpm demo` runs
  `tsc` then `vhs demo/equipe.tape`, which types the commands in a headless
  terminal and writes `demo/equipe.gif`.
- The tape aliases `equipe` to the freshly built `./dist/index.js`, so it always
  demos the current code.

## Rules of a good demo

1. **Payoff in the first 3 seconds.** Lead with the result (a `git diff`, a board
   filling in), not the setup. Captions explain; the frame shows.
2. **Under ~20 seconds.** One idea per gif.
3. **The tape upgrades each phase** — replace the commands, keep the file. The hero
   gif always shows the newest real capability. See the phase map in `equipe.tape`.

## Adding another tape

Drop `demo/<name>.tape` (with its own `Output demo/<name>.gif`) and add a script,
e.g. `"demo:try": "vhs demo/try.tape"`. Use per-phase tapes for X threads while
`equipe.tape` stays the canonical hero.
