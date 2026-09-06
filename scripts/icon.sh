#!/bin/sh
# Regenerates assets/Switchboard.icns from assets/Switchboard.svg using only
# tools that ship with macOS (swift + AppKit for a transparent render,
# sips, iconutil). Quick Look's thumbnailer paints a white background, so
# it is not used.
set -eu
cd "$(dirname "$0")/.."
WORK=$(mktemp -d)
SRC="$WORK/Switchboard.png"
swift scripts/render-icon.swift assets/Switchboard.svg "$SRC" 1024
SET="$WORK/Switchboard.iconset"
mkdir -p "$SET"
for s in 16 32 128 256 512; do
  sips -z "$s" "$s" "$SRC" --out "$SET/icon_${s}x${s}.png" >/dev/null
  d=$((s * 2))
  sips -z "$d" "$d" "$SRC" --out "$SET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$SET" -o assets/Switchboard.icns
# Raw pixels embedded in the binary for the Dock icon of a running process.
swift scripts/render-icon.swift assets/Switchboard.svg assets/icon-256.rgba 256
rm -rf "$WORK"
echo "Wrote assets/Switchboard.icns and assets/icon-256.rgba"
