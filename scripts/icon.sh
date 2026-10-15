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
# The visionOS icon is three layers the system lights in parallax and
# masks to a circle: assets/vision/{Back,Middle,Front}.svg, rendered into
# the client's asset catalog.
STACK=client/Apps/SwitchboardVision/Assets.xcassets/AppIcon.solidimagestack
for layer in Back Middle Front; do
  swift scripts/render-icon.swift "assets/vision/$layer.svg" \
    "$STACK/$layer.solidimagestacklayer/Content.imageset/$layer.png" 1024
done
rm -rf "$WORK"
echo "Wrote assets/Switchboard.icns, assets/icon-256.rgba and the visionOS icon layers"
