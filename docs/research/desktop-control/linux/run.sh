#!/bin/bash
# Live check of equipe's Linux desktop control in a throwaway Ubuntu 24.04 container.
set -u
export DEBIAN_FRONTEND=noninteractive EQUIPE_HOME=/root/.equipe NO_COLOR=1
apt-get update -qq >/dev/null && apt-get install -y -qq xvfb openbox at-spi2-core libatk-adaptor dbus-x11 zenity ca-certificates python3 procps >/dev/null 2>&1
echo "== packages installed: $(dpkg-query -W -f='${Version}' at-spi2-core) at-spi2-core"
EQ=/out/equipe
H() { python3 -I /live/helper_call.py "$@"; }

echo; echo "===== 1. bare shell: no DISPLAY, no session bus ====="
env -u DISPLAY -u WAYLAND_DISPLAY -u DBUS_SESSION_BUS_ADDRESS -u XDG_RUNTIME_DIR $EQ desktop status; echo "[exit $?]"
echo; echo "----- setup from the bare shell -----"
env -u DISPLAY -u WAYLAND_DISPLAY -u DBUS_SESSION_BUS_ADDRESS -u XDG_RUNTIME_DIR $EQ desktop setup; echo "[exit $?]"
ls -la $EQUIPE_HOME/desktop/agent-desktop/1.2.0/ 2>&1

echo; echo "===== 2. display + bus, AT-SPI not available ====="
mkdir -p /hidden && mv /usr/share/dbus-1/services/org.a11y.Bus.service /hidden/ 2>/dev/null
dbus-run-session -- bash -c '
  Xvfb :98 -screen 0 1280x800x24 >/dev/null 2>&1 & sleep 1; export DISPLAY=:98
  '"$EQ"' desktop status; echo "[exit $?]"'
mv /hidden/org.a11y.Bus.service /usr/share/dbus-1/services/

echo; echo "===== 3. a full session: Xvfb + openbox + AT-SPI + a GTK app ====="
dbus-run-session -- bash -c '
  set -u
  Xvfb :99 -screen 0 1280x800x24 >/dev/null 2>&1 & sleep 1; export DISPLAY=:99
  openbox >/dev/null 2>&1 & sleep 0.5
  /usr/libexec/at-spi-bus-launcher --launch-immediately >/dev/null 2>&1 & sleep 1
  GTK_MODULES=gail:atk-bridge zenity --entry --title "Equipe Probe" --text "Name?" > /tmp/zenity.out 2>/dev/null & ZEN=$!
  sleep 2
  '"$EQ"' desktop setup; echo "[setup exit $?]"
  echo; echo "----- status -----"
  '"$EQ"' desktop status; echo "[status exit $?]"
  echo; echo "----- helper: snapshot zenity (what a member gets) -----"
  H() { python3 -I /live/helper_call.py "$@"; }
  H apps "{\"action\":\"list\"}" | head -20
  SNAP=$(H snapshot "{\"app\":\"zenity\"}"); echo "$SNAP" | head -30
  FIELD=$(echo "$SNAP" | grep -iE "text|entry" | grep -oE "\be[0-9]+\b" | head -1)
  OK=$(echo "$SNAP" | grep -iE "button \"OK\"|\"OK\"" | grep -oE "\be[0-9]+\b" | head -1)
  echo "field=$FIELD ok=$OK"
  echo; echo "----- helper: the send gate scene for typing into $FIELD -----"
  H _scene "{\"tool\":\"act\",\"arguments\":{\"ref\":\"$FIELD\",\"action\":\"type\",\"text\":\"hello from equipe\"}}" | head -5
  echo; echo "----- helper: act type, re-read, click OK -----"
  H act "{\"ref\":\"$FIELD\",\"action\":\"type\",\"text\":\"hello from equipe\"}"
  H find "{\"app\":\"zenity\",\"text\":\"hello from equipe\"}" | head -5
  echo "(find took a snapshot of its own: refs come from a fresh snapshot)"
  SNAP=$(H snapshot "{\"app\":\"zenity\"}"); echo "$SNAP" | head -8
  OK=$(echo "$SNAP" | grep -iE "button \"OK\"" | grep -oE "\be[0-9]+\b" | head -1)
  H act "{\"ref\":\"$OK\",\"action\":\"click\"}"
  sleep 1
  if kill -0 $ZEN 2>/dev/null; then echo "zenity still open"; else echo "zenity closed; it printed: $(cat /tmp/zenity.out)"; fi
  echo; echo "----- helper: screenshot, the real pointer and keyboard (openbox: no compositor) -----"
  GTK_MODULES=gail:atk-bridge zenity --entry --title "Pointer Probe" --text "Typed?" > /tmp/zenity2.out 2>/dev/null & ZEN2=$!
  sleep 2
  H screenshot "{}"
  H list_windows "{}"
  H elements "{\"app\":\"zenity\"}"
  WIN=$(H list_windows "{}" | grep "Pointer Probe" | grep -oE "^OK w[0-9]+|^w[0-9]+" | grep -oE "w[0-9]+" | head -1)
  H screenshot "{\"window\":\"$WIN\"}"
  H screenshot "{}"
  echo "(a click outside zenity, on the bare root window, is refused before any input)"
  H click "{\"app\":\"zenity\",\"x\":1270,\"y\":790}"
  H type_text "{\"app\":\"zenity\",\"text\":\"real keys, ok\"}"
  H key "{\"app\":\"zenity\",\"keys\":\"alt+f4\"}"
  H key "{\"app\":\"zenity\",\"keys\":\"return\"}"
  sleep 1
  if kill -0 $ZEN2 2>/dev/null; then echo "zenity 2 still open"; else echo "zenity 2 closed; it printed: $(cat /tmp/zenity2.out)"; fi
  echo; echo "----- remove -----"
  '"$EQ"' desktop remove; echo "[remove exit $?]"
  sleep 0.5; pgrep -f "^/out/equipe desktop serve" >/dev/null && echo "helper still running" || echo "no helper running"
'
