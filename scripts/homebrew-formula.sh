#!/bin/sh
# Print the Homebrew formula for a release, the one equipe-labs/homebrew-tap
# serves as Formula/equipe.rb:
#
#   scripts/homebrew-formula.sh v0.26.1 SHA256SUMS > Formula/equipe.rb
#
# SHA256SUMS is the release's checksum list ("<sum>  equipe-<target>.tar.gz").
# There's no `version` line: brew reads it from the URL, and `brew audit
# --strict` calls a line that repeats it redundant.
set -eu

tag="$1"
sums="$2"
repo="${GITHUB_REPOSITORY:-equipe-labs/equipe}"
url="https://github.com/${repo}/releases/download/${tag}"

sum() {
  s="$(awk -v f="equipe-$1.tar.gz" '$2 == f || $2 == "*" f { print $1 }' "$sums")"
  [ -n "$s" ] || { echo "No checksum for equipe-$1.tar.gz in $sums" >&2; exit 1; }
  echo "$s"
}
# Up front, so a missing one stops the script rather than printing sha256 "".
mac_arm="$(sum aarch64-apple-darwin)"
mac_intel="$(sum x86_64-apple-darwin)"
linux_arm="$(sum aarch64-unknown-linux-musl)"
linux_intel="$(sum x86_64-unknown-linux-musl)"

cat <<EOF
class Equipe < Formula
  desc "Full software squad of AI agents, on your own Claude Code and Codex plans"
  homepage "https://equipe.im"
  license "MIT"

  on_macos do
    on_arm do
      url "${url}/equipe-aarch64-apple-darwin.tar.gz"
      sha256 "${mac_arm}"
    end
    on_intel do
      url "${url}/equipe-x86_64-apple-darwin.tar.gz"
      sha256 "${mac_intel}"
    end
  end

  on_linux do
    on_arm do
      url "${url}/equipe-aarch64-unknown-linux-musl.tar.gz"
      sha256 "${linux_arm}"
    end
    on_intel do
      url "${url}/equipe-x86_64-unknown-linux-musl.tar.gz"
      sha256 "${linux_intel}"
    end
  end

  def install
    bin.install "equipe"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/equipe --version")
  end
end
EOF
