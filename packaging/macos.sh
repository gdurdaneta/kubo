#!/usr/bin/env bash
# Arma kubo.app (binario universal) y lo mete en un .dmg.
#
# Corre en macOS: usa lipo, iconutil, codesign y hdiutil.
set -euo pipefail

VERSION="${1:-0.0.0}"
APP="dist/kubo.app"
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
cd "$RAIZ"

rm -rf dist/kubo.app dist/dmg
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" dist/dmg

# Un solo binario para Intel y Apple Silicon: una descarga, dos máquinas.
lipo -create -output "$APP/Contents/MacOS/kubo" \
  target/aarch64-apple-darwin/release/kubo \
  target/x86_64-apple-darwin/release/kubo
chmod +x "$APP/Contents/MacOS/kubo"

sed "s/VERSION/$VERSION/g" packaging/Info.plist > "$APP/Contents/Info.plist"

# .icns a partir de los PNG del repo.
ICONSET=dist/kubo.iconset
rm -rf "$ICONSET"; mkdir -p "$ICONSET"
cp assets/iconset/kubo-16.png   "$ICONSET/icon_16x16.png"
cp assets/iconset/kubo-32.png   "$ICONSET/icon_16x16@2x.png"
cp assets/iconset/kubo-32.png   "$ICONSET/icon_32x32.png"
cp assets/iconset/kubo-64.png   "$ICONSET/icon_32x32@2x.png"
cp assets/iconset/kubo-128.png  "$ICONSET/icon_128x128.png"
cp assets/iconset/kubo-256.png  "$ICONSET/icon_128x128@2x.png"
cp assets/iconset/kubo-256.png  "$ICONSET/icon_256x256.png"
cp assets/iconset/kubo-512.png  "$ICONSET/icon_256x256@2x.png"
cp assets/iconset/kubo-512.png  "$ICONSET/icon_512x512.png"
cp assets/iconset/kubo-1024.png "$ICONSET/icon_512x512@2x.png"
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/kubo.icns"

# Firma ad-hoc: no evita el aviso de Gatekeeper (eso pide cuenta de
# desarrollador de Apple y notarización), pero en Apple Silicon el binario
# necesita alguna firma para siquiera arrancar.
codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict "$APP" && echo "firma ad-hoc OK"

# El .dmg lleva la app y un atajo a /Applications para arrastrarla.
cp -R "$APP" dist/dmg/
ln -s /Applications dist/dmg/Applications
hdiutil create -volname "kubo $VERSION" -srcfolder dist/dmg \
  -ov -format UDZO "dist/kubo-macos-universal.dmg"

echo
echo "app: $APP"
echo "dmg: dist/kubo-macos-universal.dmg"
lipo -archs "$APP/Contents/MacOS/kubo"
