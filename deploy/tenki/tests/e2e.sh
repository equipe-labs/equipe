#!/usr/bin/env bash
# End-to-end test of the equipe-host image, backup/restore and a migration,
# on throwaway Tenki sandboxes with synthetic data only. Costs a few cents:
# two small sandboxes for well under an hour; both are terminated at the end
# (and on failure, unless KEEP=1).
#
#   deploy/tenki/tests/e2e.sh <image-ref>
#
# 1. A sandbox from the image comes up with equipe, the desktop and the timers.
# 2. Seed a squad; back up live (hot), add a ticket, back up quiesced.
# 3. Wipe the squads, restore the last snapshot, verify every ticket is back.
# 4. migrate.sh run --via stream to a second, bigger sandbox; verify there.
# 5. Sign-ins planted on the source never reach a backup or the target.
set -euo pipefail

IMAGE=${1:?usage: e2e.sh <image-ref>}
HERE=$(cd "$(dirname "$0")" && pwd)
TENKI=${TENKI:-tenki}
TAG=e2e-$(date -u +%m%d%H%M%S)
export EQUIPE_TENKI_STATE=${EQUIPE_TENKI_STATE:-$HOME/.local/state/equipe-tenki-e2e}
A="" B=""
pass() { printf '\033[32mPASS\033[0m %s\n' "$*"; }
fail() { printf '\033[31mFAIL\033[0m %s\n' "$*"; exit 1; }
tssh() { local s=$1; shift; "$TENKI" sandbox ssh --session "$s" -- -T -o LogLevel=ERROR "$*"; }
cleanup() {
  local rc=$?
  if [ "${KEEP:-0}" = 1 ]; then echo "KEEP=1: left running: ${A:-} ${B:-}"; exit "$rc"; fi
  for s in $A $B; do "$TENKI" sandbox terminate "$s" >/dev/null 2>&1 && echo "terminated $s" || true; done
  exit "$rc"
}
trap cleanup EXIT

echo "== 1. a sandbox from $IMAGE"
A=$("$TENKI" sandbox create --image "$IMAGE" --name "$TAG-a" --cpu 2 --memory-mb 4096 --disk-size-gb 15 \
  --max-duration 90m --tags equipe-e2e --wait-timeout 10m --json | python3 -c 'import sys,json; print(json.load(sys.stdin)["id"])')
echo "sandbox A: $A"
# The user's systemd manager starts a little after boot; equipe-ops waits for it.
for _ in $(seq 1 40); do tssh "$A" 'equipe-ops status --json >/dev/null 2>&1' && break; sleep 5; done
st=$(tssh "$A" 'equipe-ops status --json')
python3 - "$st" <<'PY' || fail "services on A"
import json, sys
s = json.loads(sys.argv[1])
bad = {k: v for k, v in s["services"].items() if v != "active"}
assert not bad, bad
assert s["equipe_version"], "no equipe"
print("equipe", s["equipe_version"], "image", s["image"])
PY
pass "equipe, desktop and backup timer are active"
tssh "$A" 'ss -ltnH | awk "{print \$4}" | grep -E ":590[0-9]$" | grep -v -E "^(127\.0\.0\.1|\[::1\]):"' && fail "VNC listens beyond loopback" || pass "VNC listens on loopback only"
tssh "$A" 'test -s ~/.equipe/screen/passwd && test "$(stat -c %a ~/.equipe/screen/passwd)" = 600 && test -s ~/.equipe/screen/vnc' &&
  pass "the Screen panel's VNC address and password are in place (0600)" || fail "screen files"
tssh "$A" 'google-chrome --version && restic version | head -1 && sqlite3 --version | cut -c1-20'

echo "== 2. seed, back up hot, back up quiesced"
tssh "$A" 'cat > /tmp/seed.py' <"$HERE/seed.py"
tssh "$A" 'python3 /tmp/seed.py --squad e2e --repo ~/repos/e2e --tickets 3'
# Planted sign-ins: must never travel.
tssh "$A" 'mkdir -p ~/.claude ~/.codex && echo planted > ~/.claude/.credentials.json && echo planted > ~/.codex/auth.json && echo keep > ~/.claude/settings.json'
# A local repository stands in for the bucket; the password is made on the sandbox and never printed.
tssh "$A" 'mkdir -p ~/.config/equipe-ops && umask 077 && printf "RESTIC_REPOSITORY=/home/tenki/restic-e2e\nRESTIC_PASSWORD=%s\nEQUIPE_OPS_HOST=e2e\n" "$(head -c 32 /dev/urandom | base64 | tr -d /+=)" > ~/.config/equipe-ops/backup.env'
s1=$(tssh "$A" 'equipe-ops backup --init --tag e2e' | tail -1)
[ -n "$s1" ] && pass "hot backup $s1 (daemon running)" || fail "hot backup"
tssh "$A" 'systemctl --user is-active equipe.service >/dev/null' && pass "daemon kept running through the hot backup" || fail "daemon stopped by a hot backup"
tssh "$A" 'python3 /tmp/seed.py --squad e2e --repo ~/repos/e2e --tickets 1'
want=$(tssh "$A" 'equipe-ops manifest' | python3 -c 'import sys,json; print([s["tickets"] for s in json.load(sys.stdin)["squads"] if s["name"]=="e2e"][0])')
[ "$want" = 4 ] && pass "4 tickets on A" || fail "expected 4 tickets, have $want"
s2=$(tssh "$A" 'equipe-ops backup --quiesce --tag e2e' | tail -1)
[ -n "$s2" ] && pass "quiesced backup $s2" || fail "quiesced backup"
tssh "$A" 'systemctl --user is-active equipe.service >/dev/null && ~/.local/bin/equipe daemon status >/dev/null' && pass "daemon back after the quiesced backup" || fail "daemon not back"
tssh "$A" 'set -a; . ~/.config/equipe-ops/backup.env; restic ls latest --host e2e | grep -E "credentials.json|auth.json"' && fail "a sign-in file is in the backup" || pass "no sign-in file in the backup"

echo "== 3. wipe, restore, verify"
tssh "$A" 'equipe-ops maintenance on && rm -rf ~/.equipe/squads && equipe-ops restore latest --discard-existing' >/dev/null || fail "restore"
tssh "$A" 'equipe-ops maintenance off'
got=$(tssh "$A" 'equipe-ops manifest' | python3 -c 'import sys,json; print([s["tickets"] for s in json.load(sys.stdin)["squads"] if s["name"]=="e2e"][0])')
[ "$got" = 4 ] && pass "all 4 tickets back after a restore" || fail "after restore: $got tickets"
tssh "$A" 'equipe-ops verify' >/dev/null && pass "verify: databases sound, projects are repositories" || fail "verify after restore"
tssh "$A" 'test -L ~/.local/bin/equipe -o -x ~/.local/bin/equipe && ! grep -q maintenance ~/.local/bin/equipe' && pass "equipe binary back in place after maintenance" || fail "binary"

echo "== 4. migrate to a bigger sandbox (stream)"
EQUIPE_TENKI_IMAGE=$IMAGE "$HERE/../ops/migrate.sh" run --from "$A" --name "$TAG" --disk 20 --cpu 2 --memory-mb 4096 --yes
B=$("$TENKI" sandbox get "$TAG" --json | python3 -c 'import sys,json; print(json.load(sys.stdin)["id"])')
[ "$B" != "$A" ] && pass "the name $TAG now points at the new sandbox $B" || fail "name not moved"
got=$(tssh "$B" 'equipe-ops manifest' | python3 -c 'import sys,json; print([s["tickets"] for s in json.load(sys.stdin)["squads"] if s["name"]=="e2e"][0])')
[ "$got" = 4 ] && pass "all 4 tickets on the new sandbox" || fail "on B: $got tickets"
tssh "$B" '$HOME/.local/bin/equipe daemon status >/dev/null && systemctl --user is-active equipe-desktop.service >/dev/null' && pass "daemon and desktop running on B" || fail "B services"
tssh "$B" 'test ! -e ~/.claude/.credentials.json && test ! -e ~/.codex/auth.json && test -e ~/.claude/settings.json' && pass "sign-ins stayed behind; settings came along" || fail "sign-ins travelled"
tssh "$B" 'df -BG --output=size / | tail -1'
tssh "$A" 'equipe-ops maintenance status' | grep -q since && pass "the old sandbox waits in maintenance (rollback possible)" || fail "old sandbox not in maintenance"

echo "== 5. finalize"
run=""
for f in "$EQUIPE_TENKI_STATE"/*Z-"$A".json; do [ -e "$f" ] && run=$(basename "$f" .json); done
[ -n "$run" ] || fail "no migration journal for $A" 
"$HERE/../ops/migrate.sh" finalize --run "$run" --terminate-retired
A=""
pass "end to end"
