#!/bin/bash
# Fetch the disk images of the desktop app's last few versions for one
# architecture, for scripts/desktop-dmg.sh to make update deltas from
# (EQUIPE_DMG_PREVIOUS):
#
#   scripts/desktop-previous.sh arm64 prev/      # → prev/<version>.dmg, up to 5
#   scripts/desktop-previous.sh x86_64 prev/ 3
#
# It walks the newest releases (not drafts or prereleases) and reads each one's
# appcast-<arch>.xml for the version it offers. A fast release carries the
# previous release's appcast and DMG unchanged, so a version counts once.
# Only versions older than the workspace's are kept. Each DMG is checked
# against the length its appcast gives.
#
# Deltas only make updates smaller: anything missing or failing here is a
# warning, never an error, and the release goes on with full downloads.
#
# Knobs: EQUIPE_PREVIOUS_TAGS (the tags to look at, newest first; default
# `gh release list`, which needs GH_TOKEN in CI), EQUIPE_DOWNLOAD_BASE (where
# the release assets are: <base>/<tag>/<asset>; default GitHub's).
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
repo="equipe-labs/equipe"

warn() { if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::warning::$*"; else echo "warning: $*" >&2; fi; }

arch="${1:-}"
dir="${2:-}"
count="${3:-5}"
case "$arch" in arm64 | x86_64) ;; *) echo "usage: $0 <arm64|x86_64> <dir> [count]" >&2; exit 2 ;; esac
[ -n "$dir" ] || { echo "usage: $0 <arm64|x86_64> <dir> [count]" >&2; exit 2; }
mkdir -p "$dir"

version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"
base="${EQUIPE_DOWNLOAD_BASE:-https://github.com/$repo/releases/download}"
dmg_name="equipe-$arch.dmg"
feed_name="appcast-$arch.xml"

tags="${EQUIPE_PREVIOUS_TAGS:-}"
if [ -z "$tags" ]; then
  tags="$(gh release list --repo "$repo" --exclude-drafts --exclude-pre-releases --limit 30 --json tagName -q '.[].tagName' 2>/dev/null)" || {
    warn "couldn't list the releases: no update deltas this time"
    exit 0
  }
fi

seen=" "
got=0
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
for tag in $tags; do
  [ "$got" -lt "$count" ] || break
  [ "$tag" != "v$version" ] || continue
  # A release from before the per-architecture builds has no such feed.
  curl -fsSL --max-time 60 "$base/$tag/$feed_name" -o "$work/feed.xml" 2>/dev/null || continue
  from="$(sed -n 's:.*<sparkle\:version>\([^<]*\)</sparkle\:version>.*:\1:p' "$work/feed.xml" | head -1)"
  # The feed's own DMG: the first enclosure (deltas come after it, in sparkle:deltas).
  length="$(grep -o '<enclosure [^>]*>' "$work/feed.xml" | head -1 | sed -n 's/.* length="\([0-9]*\)".*/\1/p')"
  case "$from" in "" | *[!0-9A-Za-z.+-]*) warn "$tag's $feed_name names no usable version"; continue ;; esac
  case "$seen" in *" $from "*) continue ;; esac
  seen="$seen$from "
  if [ "$from" = "$version" ] || [ "$(printf '%s\n%s\n' "$from" "$version" | sort -V | tail -1)" != "$version" ]; then
    continue
  fi
  if ! curl -fsSL --max-time 600 "$base/$tag/$dmg_name" -o "$work/$from.dmg"; then
    warn "couldn't download $dmg_name of $tag ($from): no delta from it"
    continue
  fi
  size="$(wc -c < "$work/$from.dmg" | tr -d ' ')"
  if [ -z "$length" ] || [ "$size" != "$length" ]; then
    warn "$dmg_name of $tag is $size bytes, its appcast says ${length:-nothing}: no delta from it"
    continue
  fi
  mv "$work/$from.dmg" "$dir/$from.dmg"
  echo "$dir/$from.dmg ($tag)"
  got=$((got + 1))
done
[ "$got" -gt 0 ] || echo "no earlier $arch build found: the update is a full download this time"
