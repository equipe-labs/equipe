#!/bin/sh
# Wraps the probe in an app bundle with a fixed bundle id and signs it with a
# fixed (self-signed) certificate, so its designated requirement is
# "identifier + this certificate" instead of a cdhash: a rebuilt probe keeps its
# TCC grants. The certificate lives in a keychain of its own — the login
# keychain is never touched — and nothing is marked trusted.
# usage: bundle.sh <built desktop-probe binary>
set -eu
bin=$1
app="$HOME/Applications/equipe Desktop Probe.app"
id=im.equipe.desktop-probe
cn="equipe probe signing (T-20)"
dir="$HOME/Library/Application Support/equipe-probe-t20"
kc="$dir/signing.keychain-db"
pw=equipe-probe-t20   # guards nothing secret: a throwaway key for a spike

mkdir -p "$dir"
if [ ! -f "$kc" ]; then
  cat > "$dir/cert.cnf" <<EOF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $cn
[ext]
basicConstraints = critical,CA:false
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
EOF
  openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -config "$dir/cert.cnf" \
    -keyout "$dir/key.pem" -out "$dir/cert.pem" 2>/dev/null
  openssl pkcs12 -export -inkey "$dir/key.pem" -in "$dir/cert.pem" -passout "pass:$pw" -out "$dir/cert.p12"
  security create-keychain -p "$pw" "$kc"
  security set-keychain-settings "$kc"   # no auto-lock
  security unlock-keychain -p "$pw" "$kc"
  security import "$dir/cert.p12" -k "$kc" -P "$pw" -T /usr/bin/codesign >/dev/null
  security set-key-partition-list -S apple-tool:,apple: -s -k "$pw" "$kc" >/dev/null
  rm -f "$dir/key.pem" "$dir/cert.p12"
fi
security unlock-keychain -p "$pw" "$kc"
hash=$(security find-certificate -c "$cn" -Z "$kc" | awk '/SHA-1/ {print $3}')

mkdir -p "$app/Contents/MacOS"
cp "$bin" "$app/Contents/MacOS/desktop-probe"
cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>$id</string>
<key>CFBundleName</key><string>equipe Desktop Probe</string>
<key>CFBundleExecutable</key><string>desktop-probe</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>1</string>
<key>LSUIElement</key><true/>
<key>NSScreenCaptureUsageDescription</key><string>equipe checks that it can see the screen (T-20 spike).</string>
</dict></plist>
EOF
# The requirement TCC will record: this identifier, signed by this certificate.
# codesign finds identities only through the search list: add ours for this
# one call, then put the list back exactly as it was.
before=$(security list-keychains -d user | sed 's/^ *"//; s/"$//')
set -- ; while IFS= read -r k; do set -- "$@" "$k"; done <<EOF2
$before
EOF2
security list-keychains -d user -s "$@" "$kc"
status=0
codesign --force --keychain "$kc" -s "$hash" -i "$id" \
  -r="designated => identifier \"$id\" and certificate leaf = H\"$hash\"" "$app" || status=$?
set -- ; while IFS= read -r k; do set -- "$@" "$k"; done <<EOF2
$before
EOF2
security list-keychains -d user -s "$@"
[ "$status" = 0 ] || exit "$status"
codesign -d -r- "$app" 2>&1 | grep designated
