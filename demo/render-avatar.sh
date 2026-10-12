#!/bin/sh
# Render the org avatar (1024×1024) from demo/avatar.html with headless Chrome.
set -e
cd "$(dirname "$0")"
"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
  --headless=new --disable-gpu --hide-scrollbars \
  --force-device-scale-factor=2 --window-size=512,512 --virtual-time-budget=1500 \
  --screenshot="$PWD/avatar.png" "file://$PWD/avatar.html" 2>/dev/null
echo avatar.png
