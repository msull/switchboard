#!/bin/sh
# The visionOS app on the simulator, from the terminal:
#   scripts/sim.sh build        generate the project and build for the simulator
#   scripts/sim.sh run          build, boot the simulator, install and launch
#   scripts/sim.sh shot <file>  screenshot the simulator's window into <file>
#   scripts/sim.sh log          the app's log lines from the simulator, live
# The simulator shares the Mac's network, so it discovers and pairs
# with a Switchboard running here.
set -eu
cd "$(dirname "$0")/.."
SIM="${SWITCHBOARD_SIM:-Apple Vision Pro}"
DERIVED=build/DerivedData
BUNDLE=com.sadburger.switchboard.vision
APP="$DERIVED/Build/Products/Debug-xrsimulator/Switchboard.app"

build() {
    xcodegen generate --quiet
    xcodebuild -project SwitchboardClient.xcodeproj -scheme SwitchboardVision \
        -destination "platform=visionOS Simulator,name=$SIM" \
        -derivedDataPath "$DERIVED" -quiet build
}

case "${1:-}" in
    build) build ;;
    run)
        build
        xcrun simctl boot "$SIM" 2>/dev/null || true
        open -a Simulator
        xcrun simctl install "$SIM" "$APP"
        xcrun simctl launch --terminate-running-process "$SIM" "$BUNDLE"
        ;;
    shot)
        xcrun simctl io "$SIM" screenshot "${2:?file}"
        ;;
    log)
        xcrun simctl spawn "$SIM" log stream --predicate 'process == "Switchboard"' --style compact
        ;;
    *) sed -n '2,8p' "$0"; exit 2 ;;
esac
