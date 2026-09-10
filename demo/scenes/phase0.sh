#!/usr/bin/env bash
# Scripted terminal scene for the Phase-0 hero gif (browserless: asciinema + agg).
# Runs the freshly built CLI so the gif always reflects current code.
set -euo pipefail

equipe() { node ./dist/index.js "$@"; }

# Print a shell-like prompt + command, run it, pause so it's readable.
step() {
  printf '\033[1;36m$\033[0m %s\n' "$*"
  "$@"
  echo
  sleep 1.4
}

sleep 0.6
step equipe --help
step equipe roles
sleep 1.0
