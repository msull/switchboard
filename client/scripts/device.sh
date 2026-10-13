#!/bin/sh
# The visionOS app on a headset, from the terminal, once the one-time
# Xcode setup is done (Developer Mode on the device, the device paired
# in Xcode's Devices window, the app run from Xcode once so the device
# trusts the developer):
#   scripts/device.sh build        generate the project and build for the device
#   scripts/device.sh run          build, install and launch on the device
#   scripts/device.sh devices      the devices the Mac is paired with
# The signing team comes from DEVELOPMENT_TEAM, or from the repo's
# git-ignored .env; the device from SWITCHBOARD_DEVICE (a devicectl
# identifier), or the first paired headset (the tunnel to it comes up
# on demand, so "disconnected" in the listing means nothing). The build
# is Release unless SWITCHBOARD_CONFIGURATION says Debug.
set -eu
cd "$(dirname "$0")/.."
if [ -z "${DEVELOPMENT_TEAM:-}" ] && [ -f ../.env ]; then
    DEVELOPMENT_TEAM=$(sed -n 's/^DEVELOPMENT_TEAM=//p' ../.env | tail -1)
fi
export DEVELOPMENT_TEAM="${DEVELOPMENT_TEAM:-}"
DERIVED=build/DerivedData
BUNDLE=com.sadburger.switchboard.vision
CONFIGURATION="${SWITCHBOARD_CONFIGURATION:-Release}"
APP="$DERIVED/Build/Products/$CONFIGURATION-xros/Switchboard.app"

device() {
    if [ -n "${SWITCHBOARD_DEVICE:-}" ]; then
        echo "$SWITCHBOARD_DEVICE"
        return
    fi
    LIST=$(mktemp)
    xcrun devicectl list devices --json-output "$LIST" >/dev/null 2>&1
    python3 -c 'import json, sys
for d in json.load(open(sys.argv[1]))["result"]["devices"]:
    if d["hardwareProperties"].get("platform") == "visionOS" and d["connectionProperties"].get("pairingState") == "paired":
        print(d["identifier"]); break' "$LIST"
    rm -f "$LIST"
}

build() {
    [ -n "$DEVELOPMENT_TEAM" ] || { echo "DEVELOPMENT_TEAM is not set (environment or ../.env)" >&2; exit 2; }
    xcodegen generate --quiet
    xcodebuild -project SwitchboardClient.xcodeproj -scheme SwitchboardVision \
        -destination "generic/platform=visionOS" -allowProvisioningUpdates \
        -configuration "$CONFIGURATION" -derivedDataPath "$DERIVED" -quiet build
}

case "${1:-}" in
    build) build ;;
    run)
        build
        ID=$(device)
        [ -n "$ID" ] || { echo "no paired headset; see 'devices'" >&2; exit 2; }
        xcrun devicectl device install app --device "$ID" "$APP"
        xcrun devicectl device process launch --terminate-existing --device "$ID" "$BUNDLE"
        ;;
    devices) xcrun devicectl list devices ;;
    *) sed -n '2,11p' "$0"; exit 2 ;;
esac
