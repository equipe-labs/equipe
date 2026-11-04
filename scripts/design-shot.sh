#!/bin/bash
# One state of equipe-desktop as a PNG, for the UI map's `command`
# (`equipe design flows shoot`):
#
#   scripts/design-shot.sh <light|dark> <width> <out.png> ['<steps>']
#
# <steps> are the snapshot script's verbs (the header of
# crates/desktop/src/snapshot.rs), `;`-separated, that bring the state up:
# `board;pick:2`, `design;dtab:map`. The app draws its own window to the PNG
# (`--features snapshot`): nothing is captured from the screen, and no
# permission is asked. It talks to examples/fake_daemon on a home of its own,
# so no real squad is touched: the example data the repository's map keeps.
# The person's own screens, read-only from their real squad, are
# scripts/design-shot-live.sh's — kept outside the repository.
#
# Build first:
#   cargo build -p equipe-desktop --features snapshot --bins --example fake_daemon
#
# EQUIPE_SHOT_BIN     the folder with equipe-desktop and examples/fake_daemon
#                     (default: $CARGO_TARGET_DIR/debug, else target/debug)
# EQUIPE_SHOT_HEIGHT  the window's height in points (default: 5/8 of the width, at least 600)
# EQUIPE_SHOT_WAIT    seconds the app may take, steps and all (default 60)
# EQUIPE_SNAPSHOT_SCALE  the picture's pixels per point (default 2)
# FAKE_*              passed to the fake daemon (FAKE_EMPTY=1, FAKE_BUSY=1, …)
set -euo pipefail

if [ $# -lt 3 ]; then
    echo "usage: $0 <light|dark> <width> <out.png> ['<steps>']" >&2
    exit 2
fi
mode=$1 width=$2 out=$3 steps=${4:-wait:1500}
case $mode in light | dark) ;; *) echo "mode is light or dark, not $mode" >&2; exit 2 ;; esac
case $width in '' | *[!0-9]*) echo "width is a number of points, not $width" >&2; exit 2 ;; esac

root=$(cd "$(dirname "$0")/.." && pwd)
target=${CARGO_TARGET_DIR:-$root/target}
bin=${EQUIPE_SHOT_BIN:-$target/debug}
app=$bin/equipe-desktop
daemon=$bin/examples/fake_daemon
for program in "$app" "$daemon"; do
    if [ ! -x "$program" ]; then
        echo "no $program: cargo build -p equipe-desktop --features snapshot --bins --example fake_daemon" >&2
        exit 1
    fi
done
height=${EQUIPE_SHOT_HEIGHT:-$((width * 5 / 8 > 600 ? width * 5 / 8 : 600))}
wait_secs=${EQUIPE_SHOT_WAIT:-60}

home=$(mktemp -d "${TMPDIR:-/tmp}/equipe-shot.XXXXXX")
daemon_pid='' app_pid=''
stop() {
    for pid in $app_pid $daemon_pid; do
        kill "$pid" 2>/dev/null || true
    done
    for pid in $app_pid $daemon_pid; do
        wait "$pid" 2>/dev/null || true
    done
    rm -rf "$home"
}
trap stop EXIT
trap 'exit 143' TERM INT HUP

# Tell why, from the logs, and stop.
fail() {
    echo "$1" >&2
    for log in "$home/daemon.log" "$home/app.log"; do
        [ -s "$log" ] && { echo "--- $(basename "$log")" >&2; tail -8 "$log" >&2; }
    done
    exit 1
}

export EQUIPE_HOME=$home EQUIPE_SQUAD=demo EQUIPE_SNAPSHOT_SCALE=${EQUIPE_SNAPSHOT_SCALE:-2}
echo '{"consent":"off"}' >"$home/telemetry.json"

"$daemon" >"$home/daemon.log" 2>&1 &
daemon_pid=$!
for _ in $(seq 1 100); do
    [ -S "$home/equipe.sock" ] && break
    kill -0 "$daemon_pid" 2>/dev/null || fail "the fake daemon stopped before it listened"
    sleep 0.1
done
[ -S "$home/equipe.sock" ] || fail "the fake daemon didn't listen within 10 s"

shot=$home/shot.png
EQUIPE_SCRIPT="theme:$mode;size:${width}x${height};$steps;shot:$shot;quit" "$app" >"$home/app.log" 2>&1 &
app_pid=$!
deadline=$((SECONDS + wait_secs))
while kill -0 "$app_pid" 2>/dev/null; do
    [ $SECONDS -ge $deadline ] && fail "equipe-desktop was still running after ${wait_secs} s"
    sleep 0.2
done
wait "$app_pid" 2>/dev/null || true
app_pid=''

[ -s "$shot" ] || fail "equipe-desktop drew no picture (is it built with --features snapshot?)"
mkdir -p "$(dirname "$out")"
mv "$shot" "$out"
