#!/bin/sh
# Builds the release binaries and wraps them in "Switchboard.app", installed
# to ~/Applications (or the directory given as the first argument). The
# hook helper ships next to the main binary; the app finds it by its own
# executable path.
#
# Signing: macOS ties the Accessibility grant (needed to raise Ghostty
# windows) and Keychain access to the app's code signature. The Keychain
# keys its per-item "always allow" on the signer's Team ID, and without
# one on the hash of the binary, which changes every build; so an
# identity with a Team ID (a "Developer ID Application" certificate) is
# preferred, then a self-signed "Code Signing" certificate named
# "Switchboard Dev" or "Prompt Box Dev" from Keychain Access (which keeps
# the Accessibility grant but asks for Keychain items after each build),
# then ad-hoc, which resets the grant each build. CODESIGN_IDENTITY
# overrides the search.
set -eu
cd "$(dirname "$0")/.."
DEST="${1:-$HOME/Applications}"
APP="$DEST/Switchboard.app"
ID="com.sadburger.switchboard"
VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')

# The signature the installed app had, so a change of identity below can
# clear the grants macOS keyed on it (otherwise it denies silently).
OLD_AUTHORITY=""
if [ -d "$APP" ]; then
  OLD_AUTHORITY=$(codesign -dvv "$APP" 2>&1 | sed -n 's/^Authority=//p' | head -1)
fi

cargo build --locked --release

# The build output may live outside the project (a shared target-dir in
# ~/.cargo/config.toml or CARGO_TARGET_DIR), so ask cargo where it went.
TARGET=$(cargo metadata --format-version 1 --no-deps | sed 's/.*"target_directory":"\([^"]*\)".*/\1/')
BIN="$TARGET/release"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN/switchboard" "$APP/Contents/MacOS/switchboard"
cp "$BIN/switchboard-hook" "$APP/Contents/MacOS/switchboard-hook"
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
  <key>NSMicrophoneUsageDescription</key>
  <string>Switchboard listens to your voice to dictate prompts to an agent.</string>
</dict>
</plist>
PLIST
if [ -f assets/Switchboard.icns ]; then
  cp assets/Switchboard.icns "$APP/Contents/Resources/Switchboard.icns"
  /usr/libexec/PlistBuddy -c "Add :CFBundleIconFile string Switchboard" "$APP/Contents/Info.plist"
fi
IDENTITY="${CODESIGN_IDENTITY:-}"
if [ -z "$IDENTITY" ]; then
  IDENTITIES=$(security find-identity -v -p codesigning 2>/dev/null)
  IDENTITY=$(printf '%s\n' "$IDENTITIES" | sed -n 's/.*"\(Developer ID Application: [^"]*\)".*/\1/p' | head -1)
fi
if [ -z "$IDENTITY" ]; then
  for name in "Switchboard Dev" "Prompt Box Dev"; do
    if printf '%s\n' "$IDENTITIES" | grep -q "\"$name\""; then
      IDENTITY="$name"; break
    fi
  done
fi
if [ -n "$IDENTITY" ]; then
  codesign --force --deep --sign "$IDENTITY" "$APP"
  echo "Signed with \"$IDENTITY\"; Accessibility grants persist across rebuilds."
  if ! codesign -dvv "$APP" 2>&1 | grep -q '^TeamIdentifier=[A-Z0-9]'; then
    echo "No Team ID in this identity: the Keychain asks again after each build."
  fi
  NEW_AUTHORITY=$(codesign -dvv "$APP" 2>&1 | sed -n 's/^Authority=//p' | head -1)
  if [ -n "$OLD_AUTHORITY" ] && [ "$OLD_AUTHORITY" != "$NEW_AUTHORITY" ]; then
    tccutil reset Accessibility "$ID" >/dev/null 2>&1 || true
    tccutil reset AppleEvents "$ID" >/dev/null 2>&1 || true
    echo "Signing identity changed: re-grant Accessibility when the app next asks."
  fi
else
  codesign --force --deep --sign - "$APP"
  tccutil reset Accessibility "$ID" >/dev/null 2>&1 || true
  echo "Ad-hoc signed: re-grant Accessibility on next launch."
fi
echo "Installed $APP"
