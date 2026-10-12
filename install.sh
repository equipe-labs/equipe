#!/bin/sh
# Install equipe — a full software squad of AI agents, on your own plans.
#
#   curl -fsSL https://github.com/equipe-labs/equipe/releases/latest/download/install.sh | sh
#
# Downloads the prebuilt binary for this machine from GitHub Releases, checks
# its SHA-256, and puts it in ~/.local/bin. Run it again to update.
#
#   EQUIPE_VERSION=v0.1.0      a release other than the latest
#   EQUIPE_INSTALL_DIR=DIR     somewhere other than ~/.local/bin
#   EQUIPE_NO_MODIFY_PATH=1    leave your shell profile alone
#
# Everything is inside main(), so a download cut short runs nothing.

set -eu

REPO="equipe-labs/equipe"

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
  BOLD="$(printf '\033[1m')"; DIM="$(printf '\033[2m')"; RED="$(printf '\033[31m')"
  GREEN="$(printf '\033[32m')"; RESET="$(printf '\033[0m')"
else
  BOLD=""; DIM=""; RED=""; GREEN=""; RESET=""
fi

say() { printf '%s\n' "$*"; }
ok() { printf '%s✓%s %s\n' "$GREEN" "$RESET" "$*"; }
note() { printf '  %s%s%s\n' "$DIM" "$*" "$RESET"; }
die() { printf '%s✗%s %s\n' "$RED" "$RESET" "$*" >&2; exit 1; }
has() { command -v "$1" >/dev/null 2>&1; }

target() {
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os" in
    Darwin)
      # A shell under Rosetta reports x86_64 on Apple silicon; take the native build.
      if [ "$arch" = "x86_64" ] && [ "$(sysctl -n hw.optional.arm64 2>/dev/null || echo 0)" = "1" ]; then
        arch="arm64"
      fi
      case "$arch" in
        arm64 | aarch64) echo "aarch64-apple-darwin" ;;
        x86_64) echo "x86_64-apple-darwin" ;;
        *) die "No equipe build for macOS on $arch." ;;
      esac
      ;;
    Linux)
      case "$arch" in
        x86_64 | amd64) echo "x86_64-unknown-linux-musl" ;;
        aarch64 | arm64) echo "aarch64-unknown-linux-musl" ;;
        *) die "No equipe build for Linux on $arch." ;;
      esac
      ;;
    *) die "equipe runs on macOS and Linux (this is $os). On Windows, use WSL." ;;
  esac
}

download() {
  if has curl; then
    curl --proto '=https,file' --tlsv1.2 -fsSL "$1" -o "$2"
  elif has wget; then
    wget -q --https-only "$1" -O "$2"
  else
    die "Needs curl or wget to download."
  fi
}

sha256() {
  if has sha256sum; then
    sha256sum "$1" | cut -d' ' -f1
  elif has shasum; then
    shasum -a 256 "$1" | cut -d' ' -f1
  else
    die "Needs sha256sum or shasum to check the download."
  fi
}

# The profile of the shell in use, where a PATH line belongs.
profile() {
  case "${SHELL:-}" in
    */zsh) echo "$HOME/.zshrc" ;;
    */bash) if [ "$(uname -s)" = "Darwin" ]; then echo "$HOME/.bash_profile"; else echo "$HOME/.bashrc"; fi ;;
    */fish) echo "$HOME/.config/fish/config.fish" ;;
    *) echo "$HOME/.profile" ;;
  esac
}

# A path as the person would write it: ~ for home.
tidy() {
  case "$1" in
    "$HOME"/*) echo "~${1#"$HOME"}" ;;
    *) echo "$1" ;;
  esac
}

main() {
  target="$(target)"
  version="${EQUIPE_VERSION:-latest}"
  case "$version" in
    latest) base="https://github.com/$REPO/releases/latest/download" ;;
    v*) base="https://github.com/$REPO/releases/download/$version" ;;
    *) base="https://github.com/$REPO/releases/download/v$version" ;;
  esac
  base="${EQUIPE_DOWNLOAD_BASE:-$base}"
  dir="${EQUIPE_INSTALL_DIR:-$HOME/.local/bin}"
  archive="equipe-$target.tar.gz"

  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT INT TERM

  say "${BOLD}equipe.${RESET} installing for $target"
  download "$base/$archive" "$tmp/$archive" || die "Couldn't download $base/$archive"
  download "$base/$archive.sha256" "$tmp/$archive.sha256" || die "Couldn't download the checksum for $archive"
  expected="$(cut -d' ' -f1 <"$tmp/$archive.sha256")"
  actual="$(sha256 "$tmp/$archive")"
  [ "$expected" = "$actual" ] || die "Checksum mismatch for $archive — not installing it."

  tar -xzf "$tmp/$archive" -C "$tmp"
  binary="$tmp/equipe-$target/equipe"
  [ -f "$binary" ] || die "The archive has no equipe binary."

  mkdir -p "$dir"
  previous=""
  [ -x "$dir/equipe" ] && previous="$("$dir/equipe" --version 2>/dev/null | cut -d' ' -f2 || true)"
  # Copy beside the old binary, then rename over it: a new file rather than an
  # overwritten one, which macOS would otherwise kill on its next launch.
  cp "$binary" "$dir/.equipe.new"
  chmod 755 "$dir/.equipe.new"
  mv -f "$dir/.equipe.new" "$dir/equipe"
  installed="$("$dir/equipe" --version | cut -d' ' -f2)"

  if [ -n "$previous" ] && [ "$previous" != "$installed" ]; then
    ok "Updated equipe $previous → $installed in $(tidy "$dir")"
  else
    ok "Installed equipe $installed in $(tidy "$dir")"
  fi

  case ":$PATH:" in
    *":$dir:"*) on_path=1 ;;
    *) on_path=0 ;;
  esac
  if [ "$on_path" = 0 ]; then
    profile="$(profile)"
    if [ -n "${EQUIPE_NO_MODIFY_PATH:-}" ]; then
      note "$(tidy "$dir") isn't on your PATH — add it, or run $(tidy "$dir")/equipe"
    elif [ -f "$profile" ] && grep -F "$dir" "$profile" >/dev/null 2>&1; then
      note "Open a new terminal to have equipe on your PATH ($(tidy "$profile") has it)."
    else
      mkdir -p "$(dirname "$profile")"
      case "$profile" in
        *.fish) line="fish_add_path \"$dir\"" ;;
        *) line="export PATH=\"$dir:\$PATH\"" ;;
      esac
      printf '\n# equipe\n%s\n' "$line" >>"$profile"
      ok "Added $(tidy "$dir") to your PATH in $(tidy "$profile")"
      note "Open a new terminal, or run: $line"
    fi
  fi

  if [ -S "$HOME/.equipe/equipe.sock" ] && [ -n "$previous" ]; then
    note "Your squad's daemon still runs the old build: \`equipe daemon restart\` when no one is working."
  fi

  say ""
  if has claude; then
    ok "Claude Code found — members can run on your Claude plan."
  else
    note "Claude Code isn't installed. For Claude members: https://code.claude.com/docs/en/setup"
    note "Or sign in with a ChatGPT plan: equipe login codex"
  fi
  say ""
  say "Next: open a terminal in your project and run ${BOLD}equipe${RESET}."
}

main "$@"
