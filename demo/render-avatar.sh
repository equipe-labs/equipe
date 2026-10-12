#!/bin/sh
# Render the org avatar (1024×1024) from demo/avatar.html with headless Chrome.
#   demo/render-avatar.sh            → demo/avatar.png         (the e and its chrome period)
#   demo/render-avatar.sh object     → demo/avatar-object.png  (the sculpture alone)
set -e
cd "$(dirname "$0")"
variant="${1:-mark}"
out="avatar.png"
[ "$variant" = object ] && out="avatar-object.png"
"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
  --headless=new --disable-gpu --hide-scrollbars \
  --force-device-scale-factor=2 --window-size=512,512 --virtual-time-budget=1500 \
  --screenshot="$PWD/$out" "file://$PWD/avatar.html#$variant" 2>/dev/null
echo "$out"
