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
# overrides the search. macOS may also ask for the local network again
# after the signature changes; tccutil cannot reset that grant, which
# lives under System Settings > Privacy & Security > Local Network.
set -eu
cd "$(dirname "$0")/.."
DEST="${1:-$HOME/Applications}"
APP="$DEST/Switchboard.app"
ID="com.sadburger.switchboard"
VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
# The build number a release stamps on the bundle (release.sh passes
# the version and the date); a local build carries the version alone.
BUILD="${BUNDLE_BUILD:-$VERSION}"

# The signature the installed app had, so a change of identity below can
# clear the grants macOS keyed on it (otherwise it denies silently).
OLD_AUTHORITY=""
if [ -d "$APP" ]; then
  OLD_AUTHORITY=$(codesign -dvv "$APP" 2>&1 | sed -n 's/^Authority=//p' | head -1)
fi

cargo build --locked --release --workspace

# The build output may live outside the project (a shared target-dir in
# ~/.cargo/config.toml or CARGO_TARGET_DIR), so ask cargo where it went.
TARGET=$(cargo metadata --format-version 1 --no-deps | sed 's/.*"target_directory":"\([^"]*\)".*/\1/')
BIN="$TARGET/release"

# Built and signed in a staging directory, then swapped in by rename,
# so an app that is running keeps its files: nothing of its bundle is
# overwritten or deleted underneath it. The previous bundle is kept
# aside once (a running app may still be mapping it).
STAGE="$DEST/.Switchboard.app.new"
PREVIOUS="$DEST/.Switchboard.app.previous"
rm -rf "$STAGE"
mkdir -p "$STAGE/Contents/MacOS" "$STAGE/Contents/Resources"
FINAL="$APP"
APP="$STAGE"
cp "$BIN/switchboard" "$APP/Contents/MacOS/switchboard"
cp "$BIN/switchboard-hook" "$APP/Contents/MacOS/switchboard-hook"
cp "$BIN/switchboard-env" "$APP/Contents/MacOS/switchboard-env"
cp "$BIN/switchboard-ask" "$APP/Contents/MacOS/switchboard-ask"
cp "$BIN/dispatch" "$APP/Contents/MacOS/dispatch"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>            <string>Switchboard</string>
  <key>CFBundleDisplayName</key>     <string>Switchboard</string>
  <key>CFBundleIdentifier</key>      <string>$ID</string>
  <key>CFBundleVersion</key>         <string>$BUILD</string>
  <key>CFBundleShortVersionString</key> <string>$VERSION</string>
  <key>CFBundleExecutable</key>      <string>switchboard</string>
  <key>CFBundlePackageType</key>     <string>APPL</string>
  <key>CFBundleInfoDictionaryVersion</key> <string>6.0</string>
  <key>CFBundleDevelopmentRegion</key>     <string>en</string>
  <key>LSApplicationCategoryType</key>     <string>public.app-category.developer-tools</string>
  <key>LSMinimumSystemVersion</key>  <string>13.0</string>
  <key>NSHighResolutionCapable</key> <true/>
  <key>NSAppleEventsUsageDescription</key>
  <string>Switchboard uses System Events to bring the Ghostty window of a session to the front.</string>
  <key>NSMicrophoneUsageDescription</key>
  <string>Switchboard listens to your voice to dictate prompts to an agent.</string>
  <!-- Bonjour advertising on macOS needs both: the service named and
       the reason the system shows when it asks for the local network.
       The service must match Advertisement::SERVICE in src/ports/remote.rs. -->
  <key>NSLocalNetworkUsageDescription</key>
  <string>Switchboard announces this Mac to your Vision Pro and iPad on the local network and answers the ones you pair with.</string>
  <key>NSBonjourServices</key>
  <array><string>_switchboard._tcp</string></array>
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
LOCAL_NETWORK_HINT="If the headset no longer finds this Mac, check Switchboard under System Settings > Privacy & Security > Local Network."
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
    echo "$LOCAL_NETWORK_HINT"
  fi
else
  codesign --force --deep --sign - "$APP"
  tccutil reset Accessibility "$ID" >/dev/null 2>&1 || true
  echo "Ad-hoc signed: re-grant Accessibility on next launch."
  echo "$LOCAL_NETWORK_HINT"
fi
rm -rf "$PREVIOUS"
if [ -d "$FINAL" ]; then
  mv "$FINAL" "$PREVIOUS"
fi
mv "$APP" "$FINAL"
echo "Installed $FINAL (the previous bundle is at $PREVIOUS until the next build)"
