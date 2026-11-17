#!/bin/bash
# Build the desktop app for people to download: equipe.app in a disk image
# (drag to Applications), and the Sparkle appcast its updates come from.
# One command, on a Mac:
#
#   scripts/desktop-dmg.sh                            # universal: dist/equipe.dmg (+ appcast.xml)
#   EQUIPE_DMG_ARCHS=aarch64 scripts/desktop-dmg.sh   # Apple silicon: dist/equipe-arm64.dmg (+ appcast-arm64.xml)
#   EQUIPE_DMG_ARCHS=x86_64 scripts/desktop-dmg.sh    # Intel: dist/equipe-x86_64.dmg (+ appcast-x86_64.xml)
#
# Each build has its own feed. The app reads its Mac's own (src/relaunch.rs,
# `feed_for`): a universal copy moves to its Mac's build with its next update,
# and an Intel Mac never reads the Apple silicon feed, whose items also carry
# `sparkle:hardwareRequirements arm64`, which Sparkle itself enforces.
#
# What it does with what it finds (docs/DESKTOP.md "Shipping the app"):
#   - A "Developer ID Application" identity (EQUIPE_SIGN_IDENTITY, else the
#     first in the keychain): signs with Hardened Runtime. Without one (or with
#     EQUIPE_SIGN_IDENTITY=-) the app is signed ad hoc, and macOS warns on
#     first open — the script says so.
#   - Notary credentials (EQUIPE_NOTARY_PROFILE, a `notarytool store-credentials`
#     profile; or EQUIPE_NOTARY_KEY/_KEY_ID/_ISSUER, an App Store Connect API
#     key): notarizes the app, then the DMG, and staples each its ticket.
#     Signed builds only.
#   - The update key's public half (SPARKLE_PUBLIC_ED_KEY, else
#     crates/desktop/macos/sparkle-public-ed-key): bundles Sparkle so the app
#     updates itself. With the private half too (SPARKLE_ED_PRIVATE_KEY, the
#     `generate_keys -x` file's contents, or SPARKLE_ED_KEY_FILE) it writes a
#     signed appcast for this DMG, with Sparkle's generate_appcast. Given the
#     disk images of earlier versions of the same build (EQUIPE_DMG_PREVIOUS, a
#     folder of <version>.dmg files; scripts/desktop-previous.sh fetches them),
#     it also makes binary deltas from each (EQUIPE_DMG_DELTAS at most, 5 by
#     default): equipe-<arch>-<version>-<from>.delta, a few MB where the whole
#     app is tens. Sparkle downloads the full DMG when no delta fits.
# Secrets come only from the environment or the keychain, never the repository.
#
# Other knobs: EQUIPE_DMG_OUT (dist), EQUIPE_DMG_ARCHS ("aarch64 x86_64", the
# universal build; one arch for that arch's build), EQUIPE_DMG_PREBUILT (the
# equipe-desktop binaries, one per arch in EQUIPE_DMG_ARCHS's order, built
# elsewhere — CI builds each arch on its own machine at once — to package
# instead of building), EQUIPE_DMG_PREBUILT_CLI (the `equipe` binaries the app
# carries, likewise; built here when not given), EQUIPE_APPCAST_URL (the feed
# Info.plist names; default this build's under the latest GitHub release),
# EQUIPE_DOWNLOAD_PREFIX (where the appcast says this DMG and its deltas are;
# default this version's GitHub release), EQUIPE_DMG_FORMAT (ULMO, LZMA, which
# macOS 10.15+ opens; UDZO is a third larger), EQUIPE_BUNDLE_ID
# (im.equipe.desktop; another one for a local test build, so its notification
# permission and defaults are its own and not the installed app's).
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }

# Stapling fetches the notarization ticket from Apple's CloudKit, whose
# connection drops now and then ("The network connection was lost", error 68)
# with nothing wrong: try again a few times, waiting longer each time.
staple_retry() {
  for try in 1 2 3 4 5; do
    xcrun stapler "$@" && return 0
    say "stapler $1 failed (try $try), trying again"
    sleep $((try * 15))
  done
  echo "error: stapler $* failed five times" >&2
  return 1
}
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

[ "$(uname -s)" = Darwin ] || die "the desktop app is packaged on macOS"

repo="equipe-labs/equipe"
bundle_id="${EQUIPE_BUNDLE_ID:-im.equipe.desktop}"
min_macos="12.0" # Sparkle 2.10's floor
sparkle_version="2.10.0"
sparkle_sha256="c2bf58aa8387266ac179357b1415d6f2635f044da8be41042af32425dae6da0c"

version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
[ -n "$version" ] || die "no workspace version in Cargo.toml"
out="${EQUIPE_DMG_OUT:-$root/dist}"
archs="${EQUIPE_DMG_ARCHS:-aarch64 x86_64}"
# Which build: one arch's, or both in one (universal). Its disk image, feed
# and deltas are named after it, so the three can sit in one release.
# shellcheck disable=SC2086 # one arch per word
case "$(printf '%s\n' $archs | sort -u | tr '\n' ' ')" in
  "aarch64 ") flavor=arm64 lipo_archs="arm64" ;;
  "x86_64 ") flavor=x86_64 lipo_archs="x86_64" ;;
  "aarch64 x86_64 ") flavor=universal lipo_archs="arm64 x86_64" ;;
  *) die "EQUIPE_DMG_ARCHS is aarch64, x86_64 or both, not \"$archs\"" ;;
esac
if [ "$flavor" = universal ]; then
  dmg_name="equipe.dmg" feed_name="appcast.xml"
else
  dmg_name="equipe-$flavor.dmg" feed_name="appcast-$flavor.xml"
fi
appcast_url="${EQUIPE_APPCAST_URL:-https://github.com/$repo/releases/latest/download/$feed_name}"
download_prefix="${EQUIPE_DOWNLOAD_PREFIX:-https://github.com/$repo/releases/download/v$version/}"
case "$download_prefix" in */) ;; *) download_prefix="$download_prefix/" ;; esac
dmg_format="${EQUIPE_DMG_FORMAT:-ULMO}"
max_deltas="${EQUIPE_DMG_DELTAS:-5}"
target_dir="${CARGO_TARGET_DIR:-$root/target}"
work="$target_dir/desktop-dmg"
cargo="$(command -v cargo || echo "$HOME/.cargo/bin/cargo")"
rustup="$(command -v rustup || echo "$HOME/.cargo/bin/rustup")"

# --- What we have to sign with ----------------------------------------------

identity="${EQUIPE_SIGN_IDENTITY:-}"
if [ "$identity" = "-" ]; then
  identity="" # ad hoc, asked for
elif [ -z "$identity" ]; then
  identity="$(security find-identity -v -p codesigning 2>/dev/null | sed -n 's/.*"\(Developer ID Application: .*\)"/\1/p' | head -1)"
fi
notarize=""
if [ -n "$identity" ]; then
  if [ -n "${EQUIPE_NOTARY_PROFILE:-}" ]; then
    notarize="profile"
  elif [ -n "${EQUIPE_NOTARY_KEY:-}" ] && [ -n "${EQUIPE_NOTARY_KEY_ID:-}" ] && [ -n "${EQUIPE_NOTARY_ISSUER:-}" ]; then
    notarize="key"
  fi
fi

public_key="${SPARKLE_PUBLIC_ED_KEY:-}"
key_file_in_repo="crates/desktop/macos/sparkle-public-ed-key"
if [ -z "$public_key" ] && [ -f "$key_file_in_repo" ]; then
  public_key="$(tr -d '[:space:]' < "$key_file_in_repo")"
fi

# --- Build -------------------------------------------------------------------

say "equipe desktop $version ($flavor: $archs)"
binaries=()
build_archs="$archs"
# shellcheck disable=SC2206 # one path per word
[ -n "${EQUIPE_DMG_PREBUILT:-}" ] && binaries=($EQUIPE_DMG_PREBUILT) && build_archs=""
# (Bash 3.2, the Mac's, calls an empty array unbound under `set -u`.)
for path in ${binaries[@]+"${binaries[@]}"}; do
  [ -f "$path" ] || die "no prebuilt $path"
done
for arch in $build_archs; do
  triple="$arch-apple-darwin"
  if ! "$rustup" target list --installed 2>/dev/null | grep -qx "$triple"; then
    "$rustup" target add "$triple"
  fi
  MACOSX_DEPLOYMENT_TARGET="$min_macos" CARGO_PROFILE_RELEASE_CODEGEN_UNITS="${CARGO_PROFILE_RELEASE_CODEGEN_UNITS:-16}" "$cargo" build --release --locked -p equipe-desktop --target "$triple"
  binaries+=("$target_dir/$triple/release/equipe-desktop")
done

# The `equipe` the app carries — the daemon and the command line, the app's
# own version — so the app runs on a Mac that never installed equipe.
cli_binaries=()
# shellcheck disable=SC2206 # one path per word
[ -n "${EQUIPE_DMG_PREBUILT_CLI:-}" ] && cli_binaries=($EQUIPE_DMG_PREBUILT_CLI)
for path in ${cli_binaries[@]+"${cli_binaries[@]}"}; do
  [ -f "$path" ] || die "no prebuilt $path"
done
if [ ${#cli_binaries[@]} -eq 0 ]; then
  for arch in $archs; do
    triple="$arch-apple-darwin"
    if ! "$rustup" target list --installed 2>/dev/null | grep -qx "$triple"; then
      "$rustup" target add "$triple"
    fi
    MACOSX_DEPLOYMENT_TARGET="$min_macos" "$cargo" build --release --locked -p equipe --bin equipe --target "$triple"
    cli_binaries+=("$target_dir/$triple/release/equipe")
  done
fi

rm -rf "$work"
mkdir -p "$work" "$out"
app="$work/equipe.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
lipo -create "${binaries[@]}" -output "$app/Contents/MacOS/equipe-desktop"
# In Helpers, not MacOS. The app's own executable is also the desktop helper
# (`equipe-desktop desktop serve`, started by the helper's LaunchAgent): one
# permission entry, "equipe", with the app's icon (docs/DESKTOP-CONTROL.md).
mkdir -p "$app/Contents/Helpers"
lipo -create "${cli_binaries[@]}" -output "$app/Contents/Helpers/equipe"
chmod +x "$app/Contents/Helpers/equipe"
# Exactly this build's architectures in both, never more or fewer: an Apple
# silicon build that reached an Intel Mac wouldn't open.
for bin in MacOS/equipe-desktop Helpers/equipe; do
  got="$(lipo -archs "$app/Contents/$bin" | tr ' ' '\n' | sort | tr '\n' ' ' | sed 's/ $//')"
  [ "$got" = "$lipo_archs" ] || die "$bin is \"$got\", but the $flavor build is \"$lipo_archs\" (EQUIPE_DMG_PREBUILT / EQUIPE_DMG_PREBUILT_CLI don't match EQUIPE_DMG_ARCHS)"
done
cp crates/desktop/macos/AppIcon.icns "$app/Contents/Resources/AppIcon.icns"
# The languages the app speaks, each with its name and its reasons for the
# microphone and speech recognition: macOS lists equipe under System Settings ›
# General › Language & Region › Applications, and asks in the person's language.
langs=(en de es fr ja ko zh-Hans zh-Hant)
localizations=""
for lang in "${langs[@]}"; do
  strings="crates/desktop/macos/lproj/$lang.lproj/InfoPlist.strings"
  [ -f "$strings" ] || die "no $strings"
  plutil -lint "$strings" >/dev/null || die "$strings doesn't parse"
  mkdir -p "$app/Contents/Resources/$lang.lproj"
  cp "$strings" "$app/Contents/Resources/$lang.lproj/InfoPlist.strings"
  localizations="$localizations
		<string>$lang</string>"
done

updates_plist=""
if [ -n "$public_key" ]; then
  # Sparkle, fetched once per version and checked against its known digest.
  cache="$target_dir/sparkle/$sparkle_version"
  if [ ! -d "$cache/Sparkle.framework" ]; then
    say "fetching Sparkle $sparkle_version"
    rm -rf "$cache" && mkdir -p "$cache"
    curl -fsSL "https://github.com/sparkle-project/Sparkle/releases/download/$sparkle_version/Sparkle-$sparkle_version.tar.xz" -o "$cache/sparkle.tar.xz"
    echo "$sparkle_sha256  $cache/sparkle.tar.xz" | shasum -a 256 -c - >/dev/null || die "Sparkle download doesn't match its digest"
    tar -xf "$cache/sparkle.tar.xz" -C "$cache" Sparkle.framework bin
    rm "$cache/sparkle.tar.xz"
  fi
  mkdir -p "$app/Contents/Frameworks"
  cp -R "$cache/Sparkle.framework" "$app/Contents/Frameworks/"
  # The XPC services are for sandboxed apps; this one isn't (Sparkle's docs).
  rm -rf "$app/Contents/Frameworks/Sparkle.framework/Versions/B/XPCServices" "$app/Contents/Frameworks/Sparkle.framework/XPCServices"
  updates_plist="
	<key>SUFeedURL</key>
	<string>$appcast_url</string>
	<key>SUPublicEDKey</key>
	<string>$public_key</string>
	<key>SURequireSignedFeed</key>
	<true/>
	<key>SUVerifyUpdateBeforeExtraction</key>
	<true/>
	<key>SUEnableAutomaticChecks</key>
	<true/>
	<key>SUAutomaticallyUpdate</key>
	<true/>
	<key>SUScheduledCheckInterval</key>
	<integer>3600</integer>"
fi

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>en</string>
	<key>CFBundleLocalizations</key>
	<array>$localizations
	</array>
	<key>CFBundleDisplayName</key>
	<string>equipe</string>
	<key>CFBundleExecutable</key>
	<string>equipe-desktop</string>
	<key>CFBundleIconFile</key>
	<string>AppIcon</string>
	<key>CFBundleIdentifier</key>
	<string>$bundle_id</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleName</key>
	<string>equipe</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>$version</string>
	<key>CFBundleVersion</key>
	<string>$version</string>
	<key>CFBundleURLTypes</key>
	<array>
		<dict>
			<key>CFBundleURLName</key>
			<string>$bundle_id.link</string>
			<key>CFBundleTypeRole</key>
			<string>Viewer</string>
			<key>CFBundleURLSchemes</key>
			<array>
				<string>equipe</string>
			</array>
		</dict>
	</array>
	<key>LSApplicationCategoryType</key>
	<string>public.app-category.developer-tools</string>
	<key>LSMinimumSystemVersion</key>
	<string>$min_macos</string>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>NSHumanReadableCopyright</key>
	<string>MIT License</string>
	<key>NSMicrophoneUsageDescription</key>
	<string>equipe uses the microphone when you dictate a message. The sound never leaves this Mac.</string>
	<key>NSSpeechRecognitionUsageDescription</key>
	<string>equipe turns what you say, or a recording you attach, into text on this Mac.</string>
	<key>NSAppleEventsUsageDescription</key>
	<string>equipe's members read and act in apps on this Mac for you, through equipe's own checks.</string>
	<key>NSScreenCaptureUsageDescription</key>
	<string>equipe's members see the screen when they work desktop apps for you.</string>
	<key>NSLocalNetworkUsageDescription</key>
	<string>equipe connects to your other Macs and devices on the network when you choose one.</string>
	<key>EquipeDesktopServe</key>
	<true/>$updates_plist
</dict>
</plist>
EOF
plutil -lint "$app/Contents/Info.plist" >/dev/null

# --- Sign, inside out ----------------------------------------------------------

if [ -n "$identity" ]; then
  say "signing as $identity (Hardened Runtime)"
  sign() { codesign --force --timestamp --options runtime --sign "$identity" "$@"; }
else
  say "no Developer ID identity: signing ad hoc"
  sign() { codesign --force --sign - "$@"; }
fi
# The same identifier as the command line's release build (release.yml), so
# macOS keeps one identity for `equipe` — Keychain access, Local Network, App
# Management — across updates and whichever copy runs the daemon.
sign --identifier equipe "$app/Contents/Helpers/equipe"
framework="$app/Contents/Frameworks/Sparkle.framework"
if [ -d "$framework" ]; then
  sign "$framework/Versions/B/Autoupdate"
  sign "$framework/Versions/B/Updater.app"
  sign "$framework"
fi
# The microphone, for dictating into the composer (Hardened Runtime asks for it).
entitlements="$work/equipe.entitlements"
cat > "$entitlements" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>com.apple.security.device.audio-input</key>
	<true/>
</dict>
</plist>
EOF
plutil -lint "$entitlements" >/dev/null
sign --entitlements "$entitlements" "$app"
codesign --verify --deep --strict "$app"

# --- Notarize the app ------------------------------------------------------------

# The app gets its own ticket stapled before it goes into the DMG: a copy
# dragged to Applications then opens on a Mac that is offline, where Gatekeeper
# can't look the ticket up. (Stapling the DMG covers only the DMG.) The DMG is
# notarized again below, since it is a new file with its own signature.
notarize_file() {
  if [ "$notarize" = profile ]; then
    xcrun notarytool submit "$1" --keychain-profile "$EQUIPE_NOTARY_PROFILE" --wait
  else
    xcrun notarytool submit "$1" --key "$EQUIPE_NOTARY_KEY" --key-id "$EQUIPE_NOTARY_KEY_ID" --issuer "$EQUIPE_NOTARY_ISSUER" --wait
  fi | tee "$work/notary.log"
  grep -q 'status: Accepted' "$work/notary.log" || die "notarization of $(basename "$1") wasn't accepted (xcrun notarytool log <id> says why)"
}
if [ -n "$notarize" ]; then
  say "notarizing equipe.app (this waits for Apple)"
  ditto -c -k --keepParent "$app" "$work/equipe-app.zip"
  notarize_file "$work/equipe-app.zip"
  rm "$work/equipe-app.zip"
  staple_retry staple "$app"
  staple_retry validate "$app"
  spctl --assess --type exec -vv "$app"
fi

# --- DMG -------------------------------------------------------------------------

stage="$work/dmg"
mkdir -p "$stage"
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
dmg="$out/$dmg_name"
rm -f "$dmg"
# hdiutil fails now and then on CI runners ("Resource busy") with nothing
# wrong: try again a few times before giving up.
made=""
for try in 1 2 3 4; do
  if hdiutil create -volname "equipe $version" -srcfolder "$stage" -fs HFS+ -format "$dmg_format" -ov "$dmg"; then
    made=1
    break
  fi
  say "hdiutil create failed (try $try), trying again"
  sleep $((try * 5))
done
[ -n "$made" ] || { echo "error: hdiutil create failed four times" >&2; exit 1; }
[ -n "$identity" ] && codesign --force --timestamp --sign "$identity" "$dmg"

if [ -n "$notarize" ]; then
  say "notarizing $dmg_name (this waits for Apple)"
  notarize_file "$dmg"
  staple_retry staple "$dmg"
  spctl --assess --type open --context context:primary-signature -v "$dmg"
fi

# --- Appcast -------------------------------------------------------------------

appcast=""
deltas_made=0
key_file="${SPARKLE_ED_KEY_FILE:-}"
if [ -z "$key_file" ] && [ -n "${SPARKLE_ED_PRIVATE_KEY:-}" ]; then
  key_file="$work/sparkle-private-key"
  trap 'rm -f "$work/sparkle-private-key"' EXIT
  (umask 077 && printf '%s' "$SPARKLE_ED_PRIVATE_KEY" > "$key_file")
fi
if [ -n "$public_key" ] && [ -n "$key_file" ]; then
  sparkle_bin="$target_dir/sparkle/$sparkle_version/bin"
  # generate_appcast reads the folder: this DMG under its published name, and
  # earlier versions of the same build to make deltas from. Only older ones:
  # a newer (or the same) version would become the feed's item instead.
  feed_dir="$work/appcast"
  mkdir -p "$feed_dir"
  cp "$dmg" "$feed_dir/$dmg_name"
  previous_dir="${EQUIPE_DMG_PREVIOUS:-}"
  if [ -n "$previous_dir" ] && [ -d "$previous_dir" ]; then
    for old in "$previous_dir"/*.dmg; do
      [ -f "$old" ] || continue
      from="$(basename "$old" .dmg)"
      if [ "$from" != "$version" ] && [ "$(printf '%s\n%s\n' "$from" "$version" | sort -V | tail -1)" = "$version" ]; then
        cp "$old" "$feed_dir/previous-$from.dmg"
      else
        say "skipping $old: not older than $version"
      fi
    done
  fi
  froms="$(find "$feed_dir" -name 'previous-*.dmg' -exec basename {} .dmg \; | sed 's/^previous-//' | sort -V | tr '\n' ' ')"
  say "writing $feed_name${froms:+, with deltas from $froms}"
  "$sparkle_bin/generate_appcast" --ed-key-file "$key_file" \
    --download-url-prefix "$download_prefix" \
    --full-release-notes-url "https://github.com/$repo/releases/tag/v$version" \
    --maximum-versions 1 --maximum-deltas "$max_deltas" \
    -o "$feed_dir/$feed_name" "$feed_dir"
  appcast="$out/$feed_name"
  cp "$feed_dir/$feed_name" "$appcast"
  # Deltas are named after the app (equipe<version>-<from>.delta), the same
  # for every build: named after this one instead, so all sit in one release.
  for delta in "$feed_dir"/*.delta; do
    [ -f "$delta" ] || continue
    name="$(basename "$delta")"
    case "$name" in equipe[0-9]*) ;; *) die "unexpected delta name $name" ;; esac
    ours="equipe-$flavor-${name#equipe}"
    cp "$delta" "$out/$ours"
    OLD="$download_prefix$name\"" NEW="$download_prefix$ours\"" perl -pi -e 's/\Q$ENV{OLD}\E/$ENV{NEW}/g' "$appcast"
    grep -qF "$download_prefix$ours\"" "$appcast" || die "the appcast doesn't name $ours"
    deltas_made=$((deltas_made + 1))
  done
  # Signed again after the renames (sign_update replaces the signature inside).
  "$sparkle_bin/sign_update" --ed-key-file "$key_file" "$appcast" >/dev/null
  "$sparkle_bin/sign_update" --verify --ed-key-file "$key_file" "$appcast" >/dev/null
  # What the feed must say, checked: one item, this version, this DMG, and
  # the Apple silicon requirement exactly when the build is Apple silicon's.
  [ "$(grep -c '<item>' "$appcast")" = 1 ] || die "$feed_name has more than one item"
  grep -qF "<sparkle:version>$version</sparkle:version>" "$appcast" || die "$feed_name doesn't name $version"
  grep -qF "url=\"$download_prefix$dmg_name\"" "$appcast" || die "$feed_name doesn't point at $download_prefix$dmg_name"
  if grep -q '<sparkle:hardwareRequirements>[^<]*arm64' "$appcast"; then
    [ "$flavor" = arm64 ] || die "$feed_name asks for Apple silicon, but the $flavor build runs on Intel too"
  else
    [ "$flavor" != arm64 ] || die "$feed_name doesn't ask for Apple silicon"
  fi
  ! grep -q "/equipe[0-9][^\"]*\.delta\"" "$appcast" || die "$feed_name names a delta that wasn't renamed"
fi
[ "$key_file" = "$work/sparkle-private-key" ] && rm -f "$key_file"

# --- What came out -------------------------------------------------------------

(cd "$out" && shasum -a 256 "$dmg_name" > "$dmg_name.sha256")
echo
say "built $dmg ($(du -h "$dmg" | cut -f1 | tr -d ' '), $(lipo -archs "$app/Contents/MacOS/equipe-desktop"))"
if [ -n "$identity" ]; then
  echo "    signed:     YES — $identity, Hardened Runtime"
else
  echo "    signed:     NO — ad hoc only. macOS will refuse the first open from a download"
  echo "                (System Settings › Privacy & Security › Open Anyway). See docs/DESKTOP.md."
fi
if [ -n "$notarize" ]; then
  echo "    notarized:  YES — tickets stapled to equipe.app and $dmg_name"
else
  echo "    notarized:  NO${identity:+ — set EQUIPE_NOTARY_PROFILE or EQUIPE_NOTARY_KEY/_KEY_ID/_ISSUER}"
fi
if [ -z "$public_key" ]; then
  echo "    updates:    OFF — no Sparkle public key (SPARKLE_PUBLIC_ED_KEY or $key_file_in_repo)"
elif [ -n "$appcast" ]; then
  echo "    updates:    on, feed $appcast_url (the app reads its Mac's own); signed $appcast, $deltas_made delta(s)"
else
  echo "    updates:    on, feed $appcast_url; NO appcast written (no SPARKLE_ED_PRIVATE_KEY / SPARKLE_ED_KEY_FILE)"
fi
