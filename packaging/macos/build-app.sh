#!/bin/sh
# Build mailopen.app and install it in /Applications.
#   packaging/macos/build-app.sh            -> build, bundle, sign (Apple Development if present, else ad hoc), install
#   SIGN_ID="Apple Development: ..." ./build-app.sh   to pick an identity
set -eu
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
VERSION="$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)"/\1/')"
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --release -p mailopen
OUT="$ROOT/target/macos"
APP="$OUT/mailopen.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$OUT/icon.iconset"
cp target/release/mailopen "$APP/Contents/MacOS/mailopen"
sed "s/VERSION/$VERSION/g" packaging/macos/Info.plist > "$APP/Contents/Info.plist"
swift packaging/macos/make-icon.swift "$OUT/icon-1024.png" >/dev/null
for s in 16 32 128 256 512; do
  d=$((s * 2))
  sips -z $s $s "$OUT/icon-1024.png" --out "$OUT/icon.iconset/icon_${s}x${s}.png" >/dev/null
  sips -z $d $d "$OUT/icon-1024.png" --out "$OUT/icon.iconset/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$OUT/icon.iconset" -o "$APP/Contents/Resources/mailopen.icns"
SIGN_ID="${SIGN_ID:-$(security find-identity -v -p codesigning 2>/dev/null | grep -m1 -o '"Apple Development: [^"]*"' | tr -d '"' || true)}"
if [ -n "$SIGN_ID" ]; then
  codesign --force --deep --options runtime --sign "$SIGN_ID" "$APP"
else
  codesign --force --deep --sign - "$APP"
fi
codesign --verify --deep --strict "$APP"
if [ "${INSTALL:-1}" = "1" ]; then
  rm -rf /Applications/mailopen.app
  cp -R "$APP" /Applications/mailopen.app
  echo "installed: /Applications/mailopen.app ($VERSION, signed by ${SIGN_ID:-ad hoc})"
else
  echo "built: $APP"
fi
