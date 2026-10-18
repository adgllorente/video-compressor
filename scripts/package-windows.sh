#!/usr/bin/env bash
# Genera dist/CompresorVideo-windows.zip con el .exe y ffmpeg.exe al lado.
# Desde macOS/Linux compila con mingw (brew install mingw-w64); en Windows (Git Bash) usa el target nativo.
set -euo pipefail

cd "$(dirname "$0")/.."
BIN=compresor-video
FFMPEG_URL="https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip"
TARGET=${TARGET:-x86_64-pc-windows-gnu}

cargo build --release --target "$TARGET"

DIST=dist
OUT="$DIST/Compresor de video"
CACHE="$DIST/.cache"
rm -rf "$OUT"
mkdir -p "$OUT" "$CACHE"

if [ ! -f "$CACHE/ffmpeg.exe" ]; then
  echo "Descargando ffmpeg estático para Windows…"
  curl -fL --progress-bar -o "$CACHE/ffmpeg-win.zip" "$FFMPEG_URL"
  if command -v unzip >/dev/null; then
    unzip -o -q -j "$CACHE/ffmpeg-win.zip" '*/bin/ffmpeg.exe' -d "$CACHE"
  else
    powershell -NoProfile -Command "Expand-Archive -Force '$CACHE/ffmpeg-win.zip' '$CACHE/ffmpeg-win'"
    cp "$CACHE"/ffmpeg-win/*/bin/ffmpeg.exe "$CACHE/ffmpeg.exe"
  fi
fi

cp "target/$TARGET/release/$BIN.exe" "$OUT/Compresor de video.exe"
cp "$CACHE/ffmpeg.exe" "$OUT/ffmpeg.exe"

rm -f "$DIST/CompresorVideo-windows.zip"
if command -v zip >/dev/null; then
  (cd "$DIST" && zip -q -r -X CompresorVideo-windows.zip "Compresor de video")
elif command -v 7z >/dev/null; then
  (cd "$DIST" && 7z a -tzip -bso0 CompresorVideo-windows.zip "Compresor de video")
else
  powershell -NoProfile -Command "Compress-Archive -Path \"$OUT\" -DestinationPath \"$DIST/CompresorVideo-windows.zip\""
fi
echo "Listo: $DIST/CompresorVideo-windows.zip"
