#!/bin/bash
# One state of equipe-desktop exactly as the person sees it — their own squad,
# on their own daemon, in their language and look — as a PNG for the UI map's
# real data (the design screen's "실제 데이터"):
#
#   scripts/design-shot-live.sh <out.png> ['<steps>']
#
# <steps> are the snapshot script's verbs (crates/desktop/src/snapshot.rs),
# `;`-separated, after the person's look is applied: `menu:inbox`,
# `board;pick:2`. <out.png> may hold {mode} and {width}, put in as the window
# was drawn: `.../inbox/list/default.{mode}.{width}.png`.
#
# Real data never goes into a repository: <out.png> must be outside every git
# repository — keep it in ~/.equipe/squads/<squad>/ui-map/<project>/<flow>/<screen>/,
# named as design/flows/ names its pictures. Beside it a note (.json) says
# what shot it: the app, its version and commit, the squad, language, mode
# and size; the map counts the picture stale once another version is installed.
#
# Read-only (crates/desktop/src/readonly.rs): the app attaches to the running
# daemon (it never starts one), sends only commands and requests that read,
# writes none of the person's state, never shows its window and captures
# nothing of the screen — it draws itself to the PNG. Attached, it counts as
# a screen for as long as the shot takes. The person's own files the app
# could write are compared before and after; the run fails if one changed.
#
# The build: the installed app's own when it was built with the snapshot
# feature; else this checkout's snapshot build, when it is the installed
# version. EQUIPE_SHOT_ANY_BUILD=1 shoots with the checkout's anyway: the note
# says which version it is, and the map shows it stale against the one
# installed. Build it with:
#   cargo build -p equipe-desktop --features snapshot --bin equipe-desktop
#
# EQUIPE_SHOT_BIN        the folder with a snapshot build of equipe-desktop
#                        (default: $CARGO_TARGET_DIR/debug, else target/debug)
# EQUIPE_SHOT_APP        the installed app (default: /Applications/equipe.app,
#                        else ~/Applications/equipe.app)
# EQUIPE_SHOT_WAIT       seconds the app may take, steps and all (default 60)
# EQUIPE_SNAPSHOT_SCALE  the picture's pixels per point (default 2)
set -euo pipefail

if [ $# -lt 1 ] || [ -z "$1" ]; then
    echo "usage: $0 <out.png> ['<steps>']" >&2
    exit 2
fi
out=$1 steps=${2:-wait:2500}
case $out in *.png) ;; *) echo "the picture is a .png, not $out" >&2; exit 2 ;; esac
case $steps in *quit* | *shot:*) echo "the steps bring the state up; the shot and the quit are this script's" >&2; exit 2 ;; esac

root=$(cd "$(dirname "$0")/.." && pwd)
home=${EQUIPE_HOME:-$HOME/.equipe}

# ── never into a repository ──
case $out in /*) ;; *) out=$PWD/$out ;; esac
at=$(dirname "$out")
while [ ! -d "$at" ]; do at=$(dirname "$at"); done
at=$(cd "$at" && pwd -P)
probe=$at
while :; do
    if [ -e "$probe/.git" ]; then
        echo "$out is inside the repository at $probe: real data is never kept in one." >&2
        echo "Put it in $home/squads/<squad>/ui-map/<project>/<flow>/<screen>/." >&2
        exit 1
    fi
    [ "$probe" = / ] && break
    probe=$(dirname "$probe")
done

# ── the person's daemon, running ──
socket=$home/equipe.sock
[ -S "$socket" ] || { echo "no daemon is running at $socket: the real view only attaches to one, it never starts it" >&2; exit 1; }

# ── which build ──
installed='' bundle=${EQUIPE_SHOT_APP:-}
if [ -z "$bundle" ]; then
    for candidate in /Applications/equipe.app "$HOME/Applications/equipe.app"; do
        [ -d "$candidate" ] && { bundle=$candidate; break; }
    done
fi
if [ -n "$bundle" ] && [ -f "$bundle/Contents/Info.plist" ]; then
    installed=$(/usr/bin/plutil -extract CFBundleShortVersionString raw -o - "$bundle/Contents/Info.plist" 2>/dev/null || true)
fi
# A snapshot build carries the real view's note; a release build doesn't.
snapshot_capable() { [ -x "$1" ] && LC_ALL=C grep -aq EQUIPE_SHOT_COMMIT "$1"; }
app='' version='' commit='' from=''
if [ -n "$bundle" ] && snapshot_capable "$bundle/Contents/MacOS/equipe-desktop"; then
    app=$bundle/Contents/MacOS/equipe-desktop version=$installed from="the installed app ($bundle)"
else
    target=${CARGO_TARGET_DIR:-$root/target}
    app=${EQUIPE_SHOT_BIN:-$target/debug}/equipe-desktop
    snapshot_capable "$app" || { echo "no snapshot build at $app: cargo build -p equipe-desktop --features snapshot --bin equipe-desktop" >&2; exit 1; }
    version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -1)
    commit=$(git -C "$root" rev-parse --short HEAD 2>/dev/null || true)
    [ -n "$commit" ] && [ -n "$(git -C "$root" status --porcelain --untracked-files=no 2>/dev/null)" ] && commit=$commit-dirty
    from="this checkout's snapshot build ($app)"
    if [ -n "$installed" ] && [ "$installed" != "$version" ] && [ "${EQUIPE_SHOT_ANY_BUILD:-}" != 1 ]; then
        echo "the installed app is $installed and its build has no snapshot feature; this checkout builds $version." >&2
        echo "Shoot with a snapshot build of $installed (git worktree add <dir> v$installed, build it there, EQUIPE_SHOT_BIN=<its target>/debug)" >&2
        echo "if that version has the real view, or EQUIPE_SHOT_ANY_BUILD=1 to shoot with $version (the map will show it stale)." >&2
        exit 1
    fi
fi

work=$(mktemp -d "${TMPDIR:-/tmp}/equipe-shot-live.XXXXXX")
app_pid=''
stop() {
    [ -n "$app_pid" ] && { kill "$app_pid" 2>/dev/null || true; wait "$app_pid" 2>/dev/null || true; }
    rm -rf "$work"
}
trap stop EXIT
trap 'exit 143' TERM INT HUP
# The target folder is shared with other builds: run a copy of the binary.
cp "$app" "$work/equipe-desktop"

# What of the person's this app could write, as it is now.
watched() {
    for f in desktop-target desktop-composer.json desktop-design-seen.json update-chain; do
        if [ -e "$home/$f" ]; then printf '%s %s\n' "$f" "$(shasum "$home/$f" | cut -d' ' -f1)"; else printf '%s -\n' "$f"; fi
    done
    ls -A "$home/inbox" 2>/dev/null | sed 's/^/inbox /'
}
before=$(watched)
log_lines=$(wc -l <"$home/daemon.log" 2>/dev/null || echo 0)

wait_secs=${EQUIPE_SHOT_WAIT:-60}
(
    unset EQUIPE_LANG EQUIPE_SQUAD EQUIPE_TARGET EQUIPE_SNAPSHOT EQUIPE_SNAPSHOT_MS
    export EQUIPE_HOME=$home EQUIPE_SNAPSHOT_READONLY=1 EQUIPE_SHOT_COMMIT=$commit EQUIPE_SNAPSHOT_SCALE=${EQUIPE_SNAPSHOT_SCALE:-2}
    export EQUIPE_SCRIPT="$steps;shot:$work/shot.{mode}.{width}.png;quit"
    exec "$work/equipe-desktop"
) >"$work/app.log" 2>&1 &
app_pid=$!
deadline=$((SECONDS + wait_secs))
while kill -0 "$app_pid" 2>/dev/null; do
    if [ $SECONDS -ge $deadline ]; then
        echo "equipe-desktop was still running after ${wait_secs} s" >&2
        tail -8 "$work/app.log" >&2
        exit 1
    fi
    sleep 0.2
done
wait "$app_pid" 2>/dev/null || true
app_pid=''

after=$(watched)
if [ "$before" != "$after" ]; then
    echo "something of the person's changed while shooting — this must not happen:" >&2
    diff <(echo "$before") <(echo "$after") >&2 || true
    exit 1
fi

shot=$(ls "$work"/shot.*.png 2>/dev/null | head -1 || true)
[ -n "$shot" ] && [ -s "$shot" ] || { echo "equipe-desktop drew no picture" >&2; tail -8 "$work/app.log" >&2; exit 1; }
name=$(basename "$shot" .png)
mode=$(echo "$name" | cut -d. -f2) width=$(echo "$name" | cut -d. -f3)
final=${out//\{mode\}/$mode}
final=${final//\{width\}/$width}
mkdir -p "$(dirname "$final")"
mv "$shot" "$final"
[ -f "${shot%.png}.json" ] && mv "${shot%.png}.json" "${final%.png}.json"

sent=$(grep -c '^equipe readonly: sent' "$work/app.log" || true)
kept=$(grep -c '^equipe readonly: kept back' "$work/app.log" || true)
echo "$final"
echo "  shot with $version${commit:+ ($commit)} — $from${installed:+; installed: $installed}"
echo "  to the daemon: $sent read, $kept kept back ($(grep -o 'kept back [a-z]* [a-z_.]*' "$work/app.log" | sort -u | sed 's/kept back //' | tr '\n' ',' | sed 's/,$//; s/,/, /g'))"
echo "  the daemon's log: $(( $(wc -l <"$home/daemon.log" 2>/dev/null || echo 0) - log_lines )) new lines"
