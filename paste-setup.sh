#!/bin/sh
# Make ⌃⇧V in iTerm2 paste the clipboard's screenshot into equipe — also when
# equipe runs on another machine you're connected to over SSH. Run it on the
# Mac you type on:
#
#   curl -fsSL https://github.com/equipe-labs/equipe/releases/latest/download/paste-setup.sh | sh
#
# (or `equipe paste-setup`, if equipe is installed there). `--remove` undoes it.
#
# A terminal only carries text, so a screenshot can't travel as one. ⌃⇧V runs
# a small helper on this Mac that types the image down the terminal as a
# paste (a data: URL); equipe, wherever it runs, turns it back into an image
# and attaches it to the message being written.
#
#   EQUIPE_PASTE_HOME     where the helper goes (default ~/.equipe)
#   EQUIPE_ITERM_DOMAIN   iTerm2's settings domain (default com.googlecode.iterm2)
#   EQUIPE_ITERM_APP      iTerm2 itself (default /Applications/iTerm.app)

set -eu

home="${EQUIPE_PASTE_HOME:-$HOME/.equipe}"
domain="${EQUIPE_ITERM_DOMAIN:-com.googlecode.iterm2}"
app="${EQUIPE_ITERM_APP:-/Applications/iTerm.app}"
helper="$home/paste-image"
# ⌃⇧V, as iTerm2 names it: the character (with Shift, and without) and the
# Control+Shift modifier flags.
keys="0x56-0x60000 0x76-0x60000"

say() { printf '%s\n' "$*"; }
fail() {
    printf 'equipe: %s\n' "$*" >&2
    exit 1
}

[ "$(uname -s)" = Darwin ] || fail "this sets up iTerm2 on a Mac — run it on the Mac you type on."

work=$(mktemp -d "${TMPDIR:-/tmp}/equipe-paste-setup.XXXXXX")
trap 'rm -rf "$work"' EXIT

# iTerm2's own key bindings, or its defaults when none were ever changed.
map="$work/map.plist"
if ! { defaults export "$domain" "$work/all.plist" 2>/dev/null && plutil -extract GlobalKeyMap xml1 -o "$map" "$work/all.plist" 2>/dev/null; }; then
    if [ -f "$app/Contents/Resources/DefaultGlobalKeyMap.plist" ]; then
        plutil -convert xml1 -o "$map" "$app/Contents/Resources/DefaultGlobalKeyMap.plist"
    else
        printf '<?xml version="1.0" encoding="UTF-8"?>\n<plist version="1.0"><dict/></plist>\n' >"$map"
    fi
fi

if [ "${1:-}" = "--remove" ]; then
    for key in $keys; do
        plutil -remove "$key" "$map" 2>/dev/null || true
    done
    defaults write "$domain" GlobalKeyMap "$(cat "$map")"
    rm -f "$helper"
    say "⌃⇧V is iTerm2's again."
    exit 0
fi

mkdir -p "$home"
cat >"$helper.new" <<'HELPER'
#!/bin/sh
# equipe: iTerm2 runs this for ⌃⇧V. It types the clipboard's image into the
# terminal as one paste, which equipe — on this Mac or over SSH — attaches to
# the message being written. Set up by equipe paste-setup.
exec </dev/null
PATH="$PATH:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin"
dir=$(mktemp -d "${TMPDIR:-/tmp}/equipe-paste.XXXXXX") || exit 1
trap 'rm -rf "$dir"' EXIT
image="$dir/clip.png"
if command -v pngpaste >/dev/null 2>&1; then
    pngpaste "$image" >/dev/null 2>&1 || true
fi
if [ ! -s "$image" ]; then
    osascript -e "set f to (open for access POSIX file \"$image\" with write permission)" \
        -e 'try' -e 'write (the clipboard as «class PNGf») to f' -e 'end try' -e 'close access f' >/dev/null 2>&1 || true
fi
if [ ! -s "$image" ]; then
    # Nothing to paste: say so here, and send nothing down the terminal.
    osascript -e 'beep' >/dev/null 2>&1 || true
    exit 0
fi
# A Retina screenshot is bigger than a member needs to see it, and slower to
# send: at most 2000 pixels a side, and a JPEG when it's still large.
side=$(sips -g pixelWidth -g pixelHeight "$image" 2>/dev/null | awk '/pixel/ { if ($2 > m) m = $2 } END { print m + 0 }')
if [ "$side" -gt 2000 ]; then
    sips -Z 2000 "$image" >/dev/null 2>&1 || true
fi
type=png
if [ "$(wc -c <"$image")" -gt 1500000 ] && sips -s format jpeg -s formatOptions 85 "$image" --out "$dir/clip.jpg" >/dev/null 2>&1 && [ -s "$dir/clip.jpg" ]; then
    image="$dir/clip.jpg"
    type=jpeg
fi
# Whole, or not at all: a paste that started and never ended would swallow
# what's typed next.
{
    printf '\033[200~data:image/%s;base64,' "$type"
    base64 <"$image" | tr -d '\n'
    printf '\033[201~'
} >"$dir/paste" || exit 0
cat "$dir/paste"
HELPER
chmod 755 "$helper.new"
mv "$helper.new" "$helper"

# Run Coprocess (iTerm2's action 35): what the helper prints is typed into the
# session.
escaped=$(printf '%s' "$helper" | sed 's/[\\"]/\\&/g')
for key in $keys; do
    plutil -replace "$key" -json "{\"Action\":35,\"Text\":\"/bin/sh \\\"$escaped\\\"\"}" "$map"
done
defaults write "$domain" GlobalKeyMap "$(cat "$map")"

say "✓ ⌃⇧V in iTerm2 now pastes the clipboard's screenshot into equipe — on this Mac or over SSH."
say "  Take one with ⌃⇧⌘4 (to the clipboard), then ⌃⇧V in equipe's message box."
if [ "$(defaults read "$domain" LoadPrefsFromCustomFolder 2>/dev/null || echo 0)" = 1 ]; then
    say "  iTerm2 loads its settings from a folder: save them there (Settings › General › Settings) so this stays."
fi
if [ ! -d "$app" ]; then
    say "  iTerm2 isn't in $app — this works once it's installed."
fi
