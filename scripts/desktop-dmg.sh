#!/bin/bash
# Build the desktop app for people to download: equipe.app (arm64 + x86_64),
# in equipe.dmg (drag to Applications), and the Sparkle appcast its updates
# come from. One command, on a Mac:
#
#   scripts/desktop-dmg.sh            # → dist/equipe.dmg (+ dist/appcast.xml)
#
# What it does with what it finds (docs/DESKTOP.md "Shipping the app"):
#   - A "Developer ID Application" identity (EQUIPE_SIGN_IDENTITY, else the
#     first in the keychain): signs with Hardened Runtime. Without one the app
#     is signed ad hoc, and macOS warns on first open — the script says so.
#   - Notary credentials (EQUIPE_NOTARY_PROFILE, a `notarytool store-credentials`
#     profile; or EQUIPE_NOTARY_KEY/_KEY_ID/_ISSUER, an App Store Connect API
#     key): notarizes the app, then the DMG, and staples each its ticket.
#     Signed builds only.
#   - The update key's public half (SPARKLE_PUBLIC_ED_KEY, else
#     crates/desktop/macos/sparkle-public-ed-key): bundles Sparkle so the app
#     updates itself. With the private half too (SPARKLE_ED_PRIVATE_KEY, the
#     `generate_keys -x` file's contents, or SPARKLE_ED_KEY_FILE) it writes a
#     signed appcast.xml for this DMG.
# Secrets come only from the environment or the keychain, never the repository.
#
# Other knobs: EQUIPE_DMG_OUT (dist), EQUIPE_DMG_ARCHS ("aarch64 x86_64"; one
# arch for a quicker local build), EQUIPE_DMG_PREBUILT (the per-arch
# equipe-desktop binaries, built elsewhere — CI builds each arch on its own
# machine at once — to package instead of building), EQUIPE_DMG_PREBUILT_CLI
# (the per-arch `equipe` binaries the app carries, likewise; built here when
# not given), EQUIPE_APPCAST_URL / EQUIPE_DMG_URL (where
# the app looks for updates and where the appcast says this DMG is; default the
# GitHub release of this version), EQUIPE_BUNDLE_ID (im.equipe.desktop; another
# one for a local test build, so its notification permission and defaults are
# its own and not the installed app's).
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
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
appcast_url="${EQUIPE_APPCAST_URL:-https://github.com/$repo/releases/latest/download/appcast.xml}"
dmg_url="${EQUIPE_DMG_URL:-https://github.com/$repo/releases/download/v$version/equipe.dmg}"
target_dir="${CARGO_TARGET_DIR:-$root/target}"
work="$target_dir/desktop-dmg"
cargo="$(command -v cargo || echo "$HOME/.cargo/bin/cargo")"
rustup="$(command -v rustup || echo "$HOME/.cargo/bin/rustup")"

# --- What we have to sign with ----------------------------------------------

identity="${EQUIPE_SIGN_IDENTITY:-}"
if [ -z "$identity" ]; then
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

say "equipe desktop $version ($archs)"
binaries=()
# shellcheck disable=SC2206 # one path per word
[ -n "${EQUIPE_DMG_PREBUILT:-}" ] && binaries=($EQUIPE_DMG_PREBUILT) && archs=""
# (Bash 3.2, the Mac's, calls an empty array unbound under `set -u`.)
for path in ${binaries[@]+"${binaries[@]}"}; do
  [ -f "$path" ] || die "no prebuilt $path"
done
for arch in $archs; do
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
  for arch in ${EQUIPE_DMG_ARCHS:-aarch64 x86_64}; do
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
# In Helpers, not MacOS: an `equipe` in Contents/MacOS is the desktop helper's copy.
mkdir -p "$app/Contents/Helpers"
lipo -create "${cli_binaries[@]}" -output "$app/Contents/Helpers/equipe"
chmod +x "$app/Contents/Helpers/equipe"
cp crates/desktop/macos/AppIcon.icns "$app/Contents/Resources/AppIcon.icns"
# The languages the app speaks, each with its name and its reasons for the
# microphone and speech recognition: macOS lists equipe under System Settings ›
# General › Language & Region › Applications, and asks in the person's language.
langs=(ko en ja zh-Hans zh-Hant es fr de)
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
	<string>equipe turns what you say into text on this Mac and puts it in the message box.</string>$updates_plist
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
sign "$app/Contents/Helpers/equipe"
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
  xcrun stapler staple "$app"
  xcrun stapler validate "$app"
  spctl --assess --type exec -vv "$app"
fi

# --- DMG -------------------------------------------------------------------------

stage="$work/dmg"
mkdir -p "$stage"
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
dmg="$out/equipe.dmg"
rm -f "$dmg"
hdiutil create -quiet -volname "equipe $version" -srcfolder "$stage" -fs HFS+ -format UDZO -ov "$dmg"
[ -n "$identity" ] && codesign --force --timestamp --sign "$identity" "$dmg"

if [ -n "$notarize" ]; then
  say "notarizing equipe.dmg (this waits for Apple)"
  notarize_file "$dmg"
  xcrun stapler staple "$dmg"
  spctl --assess --type open --context context:primary-signature -v "$dmg"
fi

# --- Appcast -------------------------------------------------------------------

appcast=""
key_file="${SPARKLE_ED_KEY_FILE:-}"
if [ -z "$key_file" ] && [ -n "${SPARKLE_ED_PRIVATE_KEY:-}" ]; then
  key_file="$work/sparkle-private-key"
  (umask 077 && printf '%s' "$SPARKLE_ED_PRIVATE_KEY" > "$key_file")
fi
if [ -n "$public_key" ] && [ -n "$key_file" ]; then
  sign_update="$target_dir/sparkle/$sparkle_version/bin/sign_update"
  enclosure="$("$sign_update" --ed-key-file "$key_file" "$dmg")"
  appcast="$out/appcast.xml"
  cat > "$appcast" <<EOF
<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>equipe</title>
    <link>https://github.com/$repo/releases</link>
    <item>
      <title>equipe $version</title>
      <pubDate>$(LC_ALL=C date -u '+%a, %d %b %Y %H:%M:%S +0000')</pubDate>
      <sparkle:version>$version</sparkle:version>
      <sparkle:shortVersionString>$version</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>$min_macos</sparkle:minimumSystemVersion>
      <sparkle:fullReleaseNotesLink>https://github.com/$repo/releases/tag/v$version</sparkle:fullReleaseNotesLink>
      <enclosure url="$dmg_url" type="application/octet-stream" $enclosure />
    </item>
  </channel>
</rss>
EOF
  "$sign_update" --ed-key-file "$key_file" "$appcast" >/dev/null
  "$sign_update" --verify --ed-key-file "$key_file" "$appcast" >/dev/null
fi
[ "$key_file" = "$work/sparkle-private-key" ] && rm -f "$key_file"

# --- What came out -------------------------------------------------------------

(cd "$out" && shasum -a 256 equipe.dmg > equipe.dmg.sha256)
echo
say "built $dmg ($(du -h "$dmg" | cut -f1 | tr -d ' '), $(lipo -archs "$app/Contents/MacOS/equipe-desktop"))"
if [ -n "$identity" ]; then
  echo "    signed:     YES — $identity, Hardened Runtime"
else
  echo "    signed:     NO — ad hoc only. macOS will refuse the first open from a download"
  echo "                (System Settings › Privacy & Security › Open Anyway). See docs/DESKTOP.md."
fi
if [ -n "$notarize" ]; then
  echo "    notarized:  YES — tickets stapled to equipe.app and equipe.dmg"
else
  echo "    notarized:  NO${identity:+ — set EQUIPE_NOTARY_PROFILE or EQUIPE_NOTARY_KEY/_KEY_ID/_ISSUER}"
fi
if [ -z "$public_key" ]; then
  echo "    updates:    OFF — no Sparkle public key (SPARKLE_PUBLIC_ED_KEY or $key_file_in_repo)"
elif [ -n "$appcast" ]; then
  echo "    updates:    on, feed $appcast_url; signed $appcast"
else
  echo "    updates:    on, feed $appcast_url; NO appcast written (no SPARKLE_ED_PRIVATE_KEY / SPARKLE_ED_KEY_FILE)"
fi
