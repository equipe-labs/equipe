#!/usr/bin/env bash
# Move an equipe host to a new Tenki sandbox — bigger disk, new image, new
# region — without resizing anything in place, and without losing a ticket.
#
#   migrate.sh plan     --from <session> [options]     what would happen; changes nothing
#   migrate.sh run      --from <session> [options]     do it
#   migrate.sh rollback --run <run-id>                 back to the old sandbox
#   migrate.sh finalize --run <run-id> --terminate-retired   end the old sandbox
#
# Options (plan, run):
#   --image REF          image to start from (default: $EQUIPE_TENKI_IMAGE)
#   --name NAME          the stable name the host goes by (default: the source's name)
#   --disk GB            root disk of the new sandbox, 5-100 (default: 2x what is used, at least 40)
#   --cpu N              (default: the source's)      --memory-mb MB   (default: the source's)
#   --via stream|backup  stream: data goes sandbox -> this machine -> sandbox, nothing
#                        stored on the way (default). backup: a quiesced restic backup,
#                        restored on the new sandbox (needs --secrets).
#   --secrets FILE       backup.env template with secrets:// placeholders, delivered
#                        to the new sandbox (default: ops/backup.env.example if --via backup)
#   --equipe-binary FILE install this equipe on the new sandbox instead of the image's
#   --yes                don't ask before starting
#
# Never deletes data. The old sandbox stays up, in maintenance (daemon stopped,
# data untouched), until `finalize --terminate-retired`. A failure before the
# switch puts the old sandbox back in service and leaves the new one for a look.
# Every step is journalled in ~/.local/state/equipe-tenki/<run-id>.json.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
IMAGE_DIR=$(cd "$HERE/../image" && pwd)
STATE_DIR=${EQUIPE_TENKI_STATE:-$HOME/.local/state/equipe-tenki}
TENKI=${TENKI:-tenki}
mkdir -p "$STATE_DIR"
chmod 0700 "$STATE_DIR"

say() { printf '\033[1m==>\033[0m %s\n' "$*" >&2; }
warn() { printf '\033[33mwarning:\033[0m %s\n' "$*" >&2; }
die() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }

# ssh into a sandbox (id or name); the remote command is one string.
tssh() { local s=$1; shift; "$TENKI" sandbox ssh --session "$s" -- -T -o ServerAliveInterval=15 -o LogLevel=ERROR "$*"; }
session_json() { "$TENKI" sandbox get "$1" --json; }
jget() { python3 -c 'import json,sys; d=json.load(sys.stdin);
for k in sys.argv[1].split("."): d=d.get(k) if isinstance(d,dict) else None
print("" if d is None else d)' "$1"; }

# --- the run journal ---------------------------------------------------------
RUN=""
journal() { # key value
  python3 - "$STATE_DIR/$RUN.json" "$1" "$2" <<'PY'
import json, sys, os, datetime
path, key, value = sys.argv[1:4]
d = json.load(open(path)) if os.path.exists(path) else {}
d[key] = value
d.setdefault("steps", []).append({"at": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"), key: value})
tmp = path + ".tmp"; json.dump(d, open(tmp, "w"), indent=2); os.replace(tmp, path)
PY
}
jread() { python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get(sys.argv[2],""))' "$STATE_DIR/$RUN.json" "$1"; }

# --- equipe-ops on a sandbox that predates this image -------------------------
# Copies equipe-ops into ~/.local/share/equipe-ops-boot and installs the two
# tools a stream needs; the image's own equipe-ops is used when it has one.
OPS_BOOT='$HOME/.local/share/equipe-ops-boot'
ops_cmd() { # session -> the remote command prefix for equipe-ops
  if tssh "$1" 'command -v equipe-ops >/dev/null'; then
    echo "equipe-ops"
  else
    echo "EQUIPE_OPS_LIB=$OPS_BOOT EQUIPE_OPS_ETC=$OPS_BOOT bash $OPS_BOOT/equipe-ops"
  fi
}
bootstrap_ops() {
  local s=$1
  tssh "$s" 'command -v equipe-ops >/dev/null' && return 0
  say "installing equipe-ops on $s (an older sandbox without it)"
  local f
  tssh "$s" "mkdir -p $OPS_BOOT"
  for f in usr/local/bin/equipe-ops usr/local/lib/equipe-ops/opslib.py etc/equipe-ops/backup.exclude; do
    tssh "$s" "cat > $OPS_BOOT/$(basename "$f")" <"$IMAGE_DIR/rootfs/$f"
  done
  tssh "$s" "chmod 0755 $OPS_BOOT/equipe-ops; mkdir -p $OPS_BOOT/paths.d;
    for t in zstd sqlite3; do command -v \$t >/dev/null || NEED=\"\$NEED \$t\"; done;
    [ -z \"\$NEED\" ] || { sudo -n apt-get update -q >/dev/null && sudo -n apt-get install -y -q \$NEED >/dev/null; }"
}

# --- options -------------------------------------------------------------------
FROM="" IMAGE=${EQUIPE_TENKI_IMAGE:-} NAME="" DISK="" CPU="" MEM="" VIA=stream SECRETS="" BINARY="" YES=0 TERMINATE=0
parse() {
  while [ $# -gt 0 ]; do
    case "$1" in
      --from) FROM=$2; shift ;;
      --image) IMAGE=$2; shift ;;
      --name) NAME=$2; shift ;;
      --disk) DISK=$2; shift ;;
      --cpu) CPU=$2; shift ;;
      --memory-mb) MEM=$2; shift ;;
      --via) VIA=$2; shift ;;
      --secrets) SECRETS=$2; shift ;;
      --equipe-binary) BINARY=$2; shift ;;
      --run) RUN=$2; shift ;;
      --yes) YES=1 ;;
      --terminate-retired) TERMINATE=1 ;;
      *) die "unknown option $1 (see the top of $0)" ;;
    esac
    shift
  done
}

# --- plan --------------------------------------------------------------------
SRC_ID="" SRC_NAME="" USED_GB=""
plan() {
  [ -n "$FROM" ] || die "--from <session> is required"
  [ -n "$IMAGE" ] || die "--image (or EQUIPE_TENKI_IMAGE) is required: the equipe-host image ref"
  "$TENKI" status >/dev/null 2>&1 || die "tenki is not signed in (tenki login)"
  local js
  js=$(session_json "$FROM") || die "no sandbox $FROM"
  [ "$(jget state <<<"$js")" = RUNNING ] || die "sandbox $FROM is $(jget state <<<"$js"), not RUNNING"
  SRC_ID=$(jget id <<<"$js")
  SRC_NAME=$(jget name <<<"$js")
  NAME=${NAME:-$SRC_NAME}
  CPU=${CPU:-$(jget cpu_cores <<<"$js")}
  MEM=${MEM:-$(jget memory_mb <<<"$js")}
  case "$VIA" in stream | backup) ;; *) die "--via is stream or backup" ;; esac
  if [ "$VIA" = backup ]; then
    SECRETS=${SECRETS:-$HERE/backup.env.example}
    [ -f "$SECRETS" ] || die "--via backup needs --secrets FILE"
  fi
  [ -z "$BINARY" ] || [ -x "$BINARY" ] || die "--equipe-binary $BINARY is not an executable file"
  bootstrap_ops "$SRC_ID"
  local ops
  ops=$(ops_cmd "$SRC_ID")
  # What the data takes, from the source itself.
  USED_GB=$(tssh "$SRC_ID" "$ops roots | xargs -r du -sxc -B1G 2>/dev/null | tail -1 | cut -f1")
  [ -n "$USED_GB" ] || die "could not measure the data on $SRC_ID"
  if [ -z "$DISK" ]; then
    DISK=$((USED_GB * 2 + 10))
    [ "$DISK" -ge 40 ] || DISK=40
    [ "$DISK" -le 100 ] || DISK=100
  fi
  [ "$DISK" -ge 5 ] && [ "$DISK" -le 100 ] || die "--disk is 5-100 GB on Tenki"
  [ "$DISK" -gt $((USED_GB + 5)) ] || die "a ${DISK} GB disk can't hold ${USED_GB} GB of data with room to work"
  # Tenki's published rates (tenki.cloud/pricing): vCPU $0.0504/h, $0.0162/GiB-h, disk over 5 GiB $0.000108/GiB-h.
  local hourly
  hourly=$(python3 -c "print(round($CPU*0.0504 + $MEM/1024*0.0162 + max(0,$DISK-5)*0.000108, 3))")
  cat >&2 <<EOF

  from        $SRC_NAME ($SRC_ID): ${USED_GB} GB of equipe data
  to          a new sandbox from $IMAGE
              ${CPU} vCPU, $((MEM / 1024)) GB memory, ${DISK} GB disk, sticky
              about \$${hourly}/hour while it runs (the old one keeps billing until finalize)
  via         $VIA$([ "$VIA" = backup ] && echo " (secrets from $SECRETS)")
  name        the new sandbox takes the name "$NAME"; the old one becomes "$NAME-retired-<time>"
  downtime    from stopping the old daemon to the new one answering: the copy time
              (roughly 1-3 min per GB over a stream)

EOF
}

# --- run ---------------------------------------------------------------------
DST_ID="" CUTOVER=0
on_failure() {
  local rc=$?
  [ "$rc" -ne 0 ] || return 0
  [ -n "$RUN" ] || exit "$rc"
  warn "the move stopped (exit $rc); journal: $STATE_DIR/$RUN.json"
  if [ "$CUTOVER" = 0 ] && [ -n "$SRC_ID" ]; then
    warn "putting the old sandbox back in service"
    tssh "$SRC_ID" "$(ops_cmd "$SRC_ID") maintenance off" || warn "could not: run 'equipe-ops maintenance off' on $SRC_ID yourself"
    journal source "back in service"
  fi
  [ -z "$DST_ID" ] || warn "the new sandbox $DST_ID is left as it is for a look; end it with: $TENKI sandbox terminate $DST_ID"
  exit "$rc"
}

run() {
  plan
  if [ "$YES" = 0 ]; then
    read -r -p "Start the move? [y/N] " a
    case "$a" in y | Y | yes) ;; *) die "not started" ;; esac
  fi
  RUN=$(date -u +%Y%m%dT%H%M%SZ)-$SRC_ID
  trap on_failure EXIT
  journal source_id "$SRC_ID"
  journal source_name "$SRC_NAME"
  journal name "$NAME"
  journal via "$VIA"

  say "creating the new sandbox"
  local -a extra=()
  [ -z "$SECRETS" ] || extra+=(--secret-file "$SECRETS:/home/tenki/.config/equipe-ops/backup.env")
  local js
  js=$("$TENKI" sandbox create --image "$IMAGE" --name "$NAME-new-${RUN%%-*}" --cpu "$CPU" --memory-mb "$MEM" \
    --disk-size-gb "$DISK" --sticky --tags equipe --wait-timeout 10m --json ${extra[@]+"${extra[@]}"})
  DST_ID=$(jget id <<<"$js")
  [ -n "$DST_ID" ] || die "tenki sandbox create returned no id"
  journal target_id "$DST_ID"
  say "new sandbox $DST_ID; waiting for equipe on it"
  local ok=0
  for _ in $(seq 1 60); do
    # Up means the user's systemd manager runs the daemon: it starts a while after boot.
    if tssh "$DST_ID" 'equipe-ops status --json 2>/dev/null | python3 -c "import sys,json; s=json.load(sys.stdin); sys.exit(0 if s[\"services\"][\"equipe.service\"]==\"active\" and s[\"equipe_version\"] else 1)"'; then ok=1; break; fi
    sleep 5
  done
  [ "$ok" = 1 ] || die "equipe did not come up on the new sandbox within 5 minutes (is $IMAGE an equipe-host image?)"
  tssh "$DST_ID" "mkdir -p ~/.config/equipe-ops && echo $DST_ID > ~/.config/equipe-ops/instance"

  if [ -n "$BINARY" ]; then
    say "installing $(basename "$BINARY") on the new sandbox"
    tssh "$DST_ID" 'cat > ~/.local/bin/.equipe.new && chmod 0755 ~/.local/bin/.equipe.new && mv -f ~/.local/bin/.equipe.new ~/.local/bin/equipe && ~/.local/bin/equipe --version' <"$BINARY"
  fi
  tssh "$DST_ID" 'EQUIPE_OPS_REASON=migration equipe-ops maintenance on'

  local sops
  sops=$(ops_cmd "$SRC_ID")
  say "stopping equipe on the old sandbox (downtime starts)"
  tssh "$SRC_ID" "EQUIPE_OPS_REASON=migration $sops maintenance on"
  journal source "maintenance"
  tssh "$SRC_ID" "$sops manifest" >"$STATE_DIR/$RUN.manifest.json"
  local t0=$SECONDS
  if [ "$VIA" = stream ]; then
    say "copying the data, stream: old sandbox -> here -> new sandbox"
    tssh "$SRC_ID" "$sops export-stream" | tssh "$DST_ID" 'equipe-ops import-stream' >"$STATE_DIR/$RUN.import.json" ||
      die "the copy failed (see $STATE_DIR/$RUN.import.json)"
    local a b
    a=$(tssh "$SRC_ID" 'cat ~/.local/state/equipe-ops/export.sha256')
    b=$(tssh "$DST_ID" 'cat ~/.local/state/equipe-ops/import.sha256')
    [ -n "$a" ] && [ "$a" = "$b" ] || die "the stream arrived damaged (sha256 $a sent, $b received)"
    journal stream_sha256 "$a"
  else
    say "backing up the old sandbox (quiesced)"
    local snap
    snap=$(tssh "$SRC_ID" "$sops backup --quiesce --keep-maintenance --tag migration" | tail -1)
    [ -n "$snap" ] || die "no snapshot id from the backup"
    journal snapshot "$snap"
    say "restoring snapshot $snap on the new sandbox"
    tssh "$DST_ID" "equipe-ops restore $snap --discard-existing" >"$STATE_DIR/$RUN.import.json" ||
      die "the restore failed (see $STATE_DIR/$RUN.import.json)"
  fi
  journal copy_seconds "$((SECONDS - t0))"

  say "checking the new sandbox against the old one's manifest"
  tssh "$DST_ID" 'cat > ~/.local/state/equipe-ops/source-manifest.json' <"$STATE_DIR/$RUN.manifest.json"
  tssh "$DST_ID" 'equipe-ops verify --manifest ~/.local/state/equipe-ops/source-manifest.json' >"$STATE_DIR/$RUN.verify.json" ||
    { cat "$STATE_DIR/$RUN.verify.json" >&2; die "the new sandbox does not hold the same squads"; }
  journal verified "yes"

  say "starting equipe on the new sandbox"
  tssh "$DST_ID" 'equipe-ops maintenance off'
  journal target "in service"

  say "switching names: $NAME -> the new sandbox"
  CUTOVER=1
  "$TENKI" sandbox update "$SRC_ID" --name "$NAME-retired-${RUN%%-*}" >/dev/null
  "$TENKI" sandbox update "$DST_ID" --name "$NAME" >/dev/null
  journal cutover "done"
  ssh_check "$NAME" "$DST_ID"
  say "done in $((SECONDS - t0)) s of downtime. New: $DST_ID ($NAME). Old: $SRC_ID, kept in maintenance."
  cat >&2 <<EOF

  Next:
    - sign in again on the new sandbox where needed (sign-ins are never copied):
        $TENKI sandbox ssh --session $NAME      then: claude   (/login)
    - each machine whose equipe app attaches to this host: run
        $TENKI sandbox ssh --session $NAME -- -T true
      once, so its SSH alias sbx-$NAME points at the new sandbox.
    - when you're satisfied:   $0 finalize --run $RUN --terminate-retired
    - to go back instead:      $0 rollback --run $RUN

EOF
  trap - EXIT
}

# The app reaches a host as sbx-<name> through Tenki's SSH config; make sure
# that alias now lands on the new sandbox.
ssh_check() {
  local name=$1 want=$2 got
  got=$(tssh "$name" 'cat ~/.config/equipe-ops/instance 2>/dev/null' || true)
  [ "$got" = "$want" ] || { warn "tenki sandbox ssh --session $name reaches '$got', not $want"; return 0; }
  if ssh -G "sbx-$name" >/dev/null 2>&1 && got=$(ssh -T -o BatchMode=yes -o ConnectTimeout=60 "sbx-$name" 'cat ~/.config/equipe-ops/instance' 2>/dev/null); then
    [ "$got" = "$want" ] && say "ssh sbx-$name reaches the new sandbox" || warn "ssh sbx-$name still reaches '$got': run '$TENKI sandbox ssh --session $name -- -T true' to refresh"
  fi
}

runs() {
  local f
  for f in "$STATE_DIR"/*Z-*.json; do
    case "$f" in *.manifest.json | *.import.json | *.verify.json) continue ;; esac
    [ -e "$f" ] && basename "$f" .json
  done | tr '\n' ' '
}

rollback() {
  [ -n "$RUN" ] && [ -f "$STATE_DIR/$RUN.json" ] || die "--run <run-id> (one of: $(runs))"
  SRC_ID=$(jread source_id)
  DST_ID=$(jread target_id)
  NAME=$(jread name)
  say "stopping equipe on the new sandbox $DST_ID"
  [ -z "$DST_ID" ] || tssh "$DST_ID" 'EQUIPE_OPS_REASON=rollback equipe-ops maintenance on' || warn "could not stop it"
  if [ "$(jread cutover)" = "done" ]; then
    [ -z "$DST_ID" ] || "$TENKI" sandbox update "$DST_ID" --name "$NAME-rolled-back-${RUN%%-*}" >/dev/null
    "$TENKI" sandbox update "$SRC_ID" --name "$NAME" >/dev/null
  fi
  say "putting the old sandbox $SRC_ID back in service"
  tssh "$SRC_ID" "$(ops_cmd "$SRC_ID") maintenance off"
  journal rollback "done"
  warn "anything done on the new sandbox since the move stays there only ($DST_ID, in maintenance)"
}

finalize() {
  [ -n "$RUN" ] && [ -f "$STATE_DIR/$RUN.json" ] || die "--run <run-id>"
  [ "$(jread cutover)" = "done" ] || die "this run never switched over; nothing to finalize"
  [ "$(jread rollback)" != "done" ] || die "this run was rolled back"
  [ "$TERMINATE" = 1 ] || die "finalize ends the old sandbox and its disk for good: pass --terminate-retired"
  SRC_ID=$(jread source_id)
  DST_ID=$(jread target_id)
  tssh "$DST_ID" 'equipe-ops status' >&2 || die "the new sandbox doesn't answer; not ending the old one"
  say "ending the old sandbox $SRC_ID"
  "$TENKI" sandbox terminate "$SRC_ID"
  journal finalized "done"
}

cmd=${1:-}
[ $# -eq 0 ] || shift
parse "$@"
case "$cmd" in
  plan) plan ;;
  run) run ;;
  rollback) rollback ;;
  finalize) finalize ;;
  *) sed -n '2,28p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
