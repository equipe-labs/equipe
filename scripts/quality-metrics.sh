#!/bin/sh
# How a squad's members did in two windows of time — before and after a change
# to equipe — from the squad's journal and its members' Claude Code transcripts
# on this machine (docs/quality-gate.md). It only reads, and prints; keep what
# it says out of the repository.
#
#   scripts/quality-metrics.sh --squad acme --split 2026-11-05 [--days 7]
#   scripts/quality-metrics.sh --squad acme --a 2026-10-01..2026-10-15 --b 2026-10-15..2026-11-01
#   scripts/quality-metrics.sh --squad acme --versions 0.103.40 0.103.50
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
exec cargo run -q --manifest-path "$root/Cargo.toml" -p equipe-core --example quality_metrics -- --repo "$root" "$@"
