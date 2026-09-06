#!/bin/sh
# Builds the release binaries and wraps them in "Switchboard.app", installed
# to ~/Applications (or the directory given as the first argument). The
# hook helper ships next to the main binary; the app finds it by its own
# executable path.
#
# Signing: macOS ties the Accessibility grant (needed to raise Ghostty
# windows) to the app's code signature. With a real identity (set
# CODESIGN_IDENTITY, or a "Code Signing" certificate named "Switchboard
# Dev" or "Prompt Box Dev" in Keychain Access) the grant survives rebuilds.
# Without one the app is ad-hoc signed and the grant is reset each build.
set -eu
cd "$(dirname "$0")/.."
DEST="${1:-$HOME/Applications}"
APP="$DEST/Switchboard.app"
ID="com.sadburger.switchboard"
VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')

cargo build --locked --release

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/switchboard "$APP/Contents/MacOS/switchboard"
cp target/release/switchboard-hook "$APP/Contents/MacOS/switchboard-hook"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>            <string>Switchboard</string>
  <key>CFBundleDisplayName</key>     <string>Switchboard</string>
  <key>CFBundleIdentifier</key>      <string>$ID</string>
  <key>CFBundleVersion</key>         <string>$VERSION</string>
  <key>CFBundleShortVersionString</key> <string>$VERSION</string>
  <key>CFBundleExecutable</key>      <string>switchboard</string>
  <key>CFBundlePackageType</key>     <string>APPL</string>
  <key>LSMinimumSystemVersion</key>  <string>13.0</string>
  <key>NSHighResolutionCapable</key> <true/>
  <key>NSAppleEventsUsageDescription</key>
  <string>Switchboard raises the terminal window of a session you return to.</string>
</dict>
</plist>
PLIST
if [ -f assets/Switchboard.icns ]; then
  cp assets/Switchboard.icns "$APP/Contents/Resources/Switchboard.icns"
  /usr/libexec/PlistBuddy -c "Add :CFBundleIconFile string Switchboard" "$APP/Contents/Info.plist"
fi
IDENTITY="${CODESIGN_IDENTITY:-}"
if [ -z "$IDENTITY" ]; then
  for name in "Switchboard Dev" "Prompt Box Dev"; do
    if security find-identity -v -p codesigning 2>/dev/null | grep -q "\"$name\""; then
      IDENTITY="$name"; break
    fi
  done
fi
if [ -n "$IDENTITY" ]; then
  codesign --force --deep --sign "$IDENTITY" "$APP"
  echo "Signed with \"$IDENTITY\"; Accessibility grants persist across rebuilds."
else
  codesign --force --deep --sign - "$APP"
  tccutil reset Accessibility "$ID" >/dev/null 2>&1 || true
  echo "Ad-hoc signed: re-grant Accessibility on next launch."
fi
echo "Installed $APP"
