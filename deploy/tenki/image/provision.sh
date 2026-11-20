#!/bin/sh
# Turn a stock Tenki `sandbox` image (Ubuntu 24.04, user `tenki`) into an
# equipe host: a desktop session reachable only from inside the machine, Chrome,
# backup tools, the pinned equipe release, equipe-ops, and the user services
# that start them at boot.
#
# Runs once while a template image is built (template/render.py), and can be
# run again on a live sandbox from a checkout to bring it up to this layout:
#
#   sudo sh deploy/tenki/image/provision.sh
#
# Idempotent: every step checks before it changes anything. Nothing here reads,
# writes or prints a secret; credentials reach a sandbox only at create time,
# as Tenki secret files (see ops/backup.env.example).
set -eu

if [ "$(id -u)" -ne 0 ]; then
  exec sudo -n sh "$0" "$@"
fi

SRC=$(cd "$(dirname "$0")" && pwd)
USER_NAME=tenki
USER_HOME=/home/$USER_NAME
# shellcheck source=deploy/tenki/image/versions.env
. "$SRC/versions.env"
export DEBIAN_FRONTEND=noninteractive

log() { printf '[provision] %s\n' "$*"; }
die() { printf '[provision] error: %s\n' "$*" >&2; exit 1; }

id "$USER_NAME" >/dev/null 2>&1 || die "this image has no user $USER_NAME"

arch=$(uname -m)
case "$arch" in
  x86_64) equipe_target=x86_64-unknown-linux-musl; equipe_sha=$EQUIPE_SHA256_X86_64 ;;
  aarch64 | arm64) equipe_target=aarch64-unknown-linux-musl; equipe_sha=$EQUIPE_SHA256_AARCH64 ;;
  *) die "unsupported architecture $arch" ;;
esac

# --- Packages from the Ubuntu archive ------------------------------------
log "apt packages"
apt-get update -q
apt-get install -y -q --no-install-recommends \
  ca-certificates curl gnupg jq sqlite3 restic zstd rsync procps psmisc \
  dbus-user-session dbus-x11 xauth x11-utils x11-xserver-utils \
  xfce4-session xfwm4 xfce4-panel xfdesktop4 xfce4-settings xfce4-terminal thunar mousepad \
  tigervnc-standalone-server tigervnc-tools tigervnc-scraping-server \
  at-spi2-core libatk-adaptor libgail-common \
  fonts-noto-core fonts-noto-cjk fonts-noto-color-emoji

# --- Google Chrome, from Google's signed repository ------------------------
keyring=/etc/apt/keyrings/google-linux.gpg
if [ ! -s "$keyring" ]; then
  log "Google's signing key"
  install -d -m 0755 /etc/apt/keyrings
  tmpkey=$(mktemp)
  curl -fsSL https://dl.google.com/linux/linux_signing_key.pub -o "$tmpkey"
  got=$(gpg --show-keys --with-colons --fingerprint "$tmpkey" | awk -F: '$1 == "fpr" {print $10}' | tr '\n' ' ')
  case " $got " in
    *" $GOOGLE_LINUX_SIGNING_KEY_FPR "*) ;;
    *) rm -f "$tmpkey"; die "Google's key fingerprint is not the pinned one ($got)" ;;
  esac
  gpg --dearmor <"$tmpkey" >"$keyring"
  rm -f "$tmpkey"
fi
echo "deb [arch=amd64 signed-by=$keyring] https://dl.google.com/linux/chrome/deb/ stable main" >/etc/apt/sources.list.d/google-chrome.list
if [ "$arch" = x86_64 ] && ! command -v google-chrome >/dev/null 2>&1; then
  log "Google Chrome"
  apt-get update -q
  apt-get install -y -q --no-install-recommends google-chrome-stable
fi

# --- equipe, the pinned release -------------------------------------------
want="equipe $EQUIPE_VERSION"
bin=$USER_HOME/.local/bin/equipe
have=$("$bin" --version 2>/dev/null || true)
if [ "$have" != "$want" ]; then
  log "equipe $EQUIPE_VERSION ($equipe_target)"
  tmp=$(mktemp -d)
  archive="equipe-$equipe_target.tar.gz"
  curl -fsSL "https://github.com/equipe-labs/equipe/releases/download/v$EQUIPE_VERSION/$archive" -o "$tmp/$archive"
  echo "$equipe_sha  $tmp/$archive" | sha256sum -c --quiet - || die "checksum mismatch for $archive"
  tar -xzf "$tmp/$archive" -C "$tmp"
  install -d -o "$USER_NAME" -g "$USER_NAME" "$USER_HOME/.local" "$USER_HOME/.local/bin"
  # Beside the old one, then renamed over it, as install.sh does.
  install -o "$USER_NAME" -g "$USER_NAME" -m 0755 "$tmp/equipe-$equipe_target/equipe" "$USER_HOME/.local/bin/.equipe.new"
  mv -f "$USER_HOME/.local/bin/.equipe.new" "$bin"
  rm -rf "$tmp"
  [ "$("$bin" --version)" = "$want" ] || die "installed equipe does not report $want"
fi

# --- equipe-ops and its system files --------------------------------------
log "equipe-ops"
cp -R "$SRC/rootfs/." /
# Files only: directories keep 0755.
(cd "$SRC/rootfs" && find . -type f) | while read -r f; do chmod 0644 "/${f#./}"; done
chmod 0755 /usr/local/bin/equipe-ops /usr/local/lib/equipe-ops/desktop-session
install -d -m 0755 /etc/equipe-ops/paths.d

# What this image is, for `equipe-ops status` and for support.
deploy_rev=$(cat "$SRC/REVISION" 2>/dev/null || echo unknown)
cat >/etc/equipe-ops/image.json <<EOF
{"equipe": "$EQUIPE_VERSION", "deploy_revision": "$deploy_rev", "built_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)", "arch": "$arch"}
EOF

# --- The user's services ---------------------------------------------------
# Linger keeps the user manager (and so the daemon) running with nobody
# signed in, which is the normal state of a sandbox.
install -d -m 0755 /var/lib/systemd/linger
touch "/var/lib/systemd/linger/$USER_NAME"

as_user() { runuser -u "$USER_NAME" -- "$@"; }
unit_dir=$USER_HOME/.config/systemd/user
as_user mkdir -p "$unit_dir/default.target.wants" "$unit_dir/timers.target.wants" "$USER_HOME/.equipe"
# The daemon's unit is the one `equipe daemon install` writes on Linux
# (crates/cli/src/daemon.rs), so `equipe daemon status` recognises it and a
# later `equipe daemon install` rewrites it in place.
service_path="$USER_HOME/.local/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
as_user sh -c "cat >'$unit_dir/equipe.service'" <<EOF
[Unit]
Description=equipe — the squad's daemon

[Service]
ExecStart=$bin daemon run
Environment=PATH=$service_path
Restart=on-failure
StandardOutput=append:$USER_HOME/.equipe/daemon.log
StandardError=append:$USER_HOME/.equipe/daemon.log

[Install]
WantedBy=default.target
EOF
as_user ln -sfn ../equipe.service "$unit_dir/default.target.wants/equipe.service"
# The image's own user units live in /etc/systemd/user; enable them for tenki.
as_user ln -sfn /etc/systemd/user/equipe-desktop.service "$unit_dir/default.target.wants/equipe-desktop.service"
as_user ln -sfn /etc/systemd/user/equipe-backup.timer "$unit_dir/timers.target.wants/equipe-backup.timer"

# The desktop: compositing off (full-screen captures need it), accessibility on.
xfconf=$USER_HOME/.config/xfce4/xfconf/xfce-perchannel-xml
if [ ! -f "$xfconf/xfwm4.xml" ]; then
  as_user mkdir -p "$xfconf"
  as_user sh -c "cat >'$xfconf/xfwm4.xml'" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<channel name="xfwm4" version="1.0">
  <property name="general" type="empty">
    <property name="use_compositing" type="bool" value="false"/>
  </property>
</channel>
EOF
fi

# --- Leave nothing per-machine in the image --------------------------------
apt-get clean
rm -rf /var/lib/apt/lists/*
log "done: $want, $(google-chrome --version 2>/dev/null || echo 'no Chrome'), $(restic version | cut -d' ' -f1-2)"
