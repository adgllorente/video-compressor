#!/usr/bin/env bash
# Genera dist/CompresorVideo-macos.zip con la app y ffmpeg dentro (Apple Silicon).
set -euo pipefail

cd "$(dirname "$0")/.."
APP_NAME="Compresor de vídeo"
BIN=compresor-video
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
FFMPEG_URL="https://www.osxexperts.net/ffmpeg9arm.zip"

cargo build --release --target aarch64-apple-darwin

DIST=dist
APP="$DIST/$APP_NAME.app"
CACHE="$DIST/.cache"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$CACHE"

if [ ! -x "$CACHE/ffmpeg-macos-arm64" ]; then
  echo "Descargando ffmpeg estático para macOS…"
  curl -fL --progress-bar -o "$CACHE/ffmpeg-mac.zip" "$FFMPEG_URL"
  unzip -o -q "$CACHE/ffmpeg-mac.zip" ffmpeg -d "$CACHE"
  mv "$CACHE/ffmpeg" "$CACHE/ffmpeg-macos-arm64"
  chmod +x "$CACHE/ffmpeg-macos-arm64"
fi

cp "target/aarch64-apple-darwin/release/$BIN" "$APP/Contents/MacOS/$BIN"
cp "$CACHE/ffmpeg-macos-arm64" "$APP/Contents/MacOS/ffmpeg"

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$APP_NAME</string>
  <key>CFBundleDisplayName</key><string>$APP_NAME</string>
  <key>CFBundleIdentifier</key><string>es.personal.compresor-video</string>
  <key>CFBundleExecutable</key><string>$BIN</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF

if [ -f assets/AppIcon.icns ]; then
  cp assets/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
  /usr/libexec/PlistBuddy -c "Add :CFBundleIconFile string AppIcon" "$APP/Contents/Info.plist"
fi

codesign --force --sign - "$APP/Contents/MacOS/ffmpeg"
codesign --force --sign - "$APP"

rm -f "$DIST/CompresorVideo-macos.zip"
ditto -c -k --keepParent "$APP" "$DIST/CompresorVideo-macos.zip"
echo "Listo: $DIST/CompresorVideo-macos.zip"
