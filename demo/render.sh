#!/usr/bin/env bash
# Regenerate demo/equipe.gif from the current code.
#
#   demo/render.sh             VHS, driven by demo/equipe.tape   (brew install vhs)
#   demo/render.sh --headless  asciinema + agg, driven by demo/scenes/phase0.sh
#                              (brew install asciinema agg) — for sandboxes without
#                              the headless browser VHS needs
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release -p equipe

if [[ "${1:-}" == "--headless" ]]; then
  asciinema rec --overwrite -c 'bash demo/scenes/phase0.sh' demo/equipe.cast
  agg --theme dracula --font-size 20 demo/equipe.cast demo/equipe.gif
else
  vhs demo/equipe.tape
fi
