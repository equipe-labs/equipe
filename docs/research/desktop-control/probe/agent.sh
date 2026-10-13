#!/bin/sh
# Loads the probe as a throwaway LaunchAgent in the person's GUI session, the way
# `equipe daemon install` loads the daemon, runs it once and prints its log.
# usage: agent.sh <probe binary> <mode> <out dir> [aqua|any]
set -eu
exe=$1; mode=$2; out=$3; limit=${4:-aqua}
label=im.equipe.probe-t20
uid=$(id -u)
mkdir -p "$out"
plist="$out/$label.plist"
log="$out/agent.log"
limit_xml=""
[ "$limit" = aqua ] && limit_xml="<key>LimitLoadToSessionType</key><string>Aqua</string>"
cat > "$plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$label</string>
  <key>ProgramArguments</key>
  <array><string>$exe</string><string>$mode</string><string>$out</string></array>
  <key>RunAtLoad</key><true/>
  $limit_xml
  <key>StandardOutPath</key><string>$log</string>
  <key>StandardErrorPath</key><string>$log</string>
</dict>
</plist>
PLIST
: > "$log"
launchctl bootout "gui/$uid/$label" 2>/dev/null || true
launchctl bootstrap "gui/$uid" "$plist"
for _ in $(seq 1 120); do grep -q "==== done" "$log" && break; sleep 0.5; done
launchctl bootout "gui/$uid/$label" 2>/dev/null || true
cat "$log"
