#!/usr/bin/env bash
# Build dist/Pinch.app: the three binaries in one signed bundle.
#
# macOS only delivers actionable notifications to a process that runs from a
# signed .app bundle, and a bundle is also what lets `pinchd` and `pinch-ui`
# share one identity (and one "no Dock icon" setting). The CLI lives inside the
# bundle too, so it can always find `pinchd` next to itself.
#
#   scripts/bundle.sh                 # release build (slow the first time)
#   PROFILE=debug scripts/bundle.sh   # debug build, for iterating
set -euo pipefail

cd "$(dirname "$0")/.."
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-13.0}"
PROFILE="${PROFILE:-release}"
VERSION=$(sed -n '/\[workspace.package\]/,/^\[/s/^version = "\([^"]*\)"/\1/p' Cargo.toml)
APP="dist/Pinch.app"
MACOS="$APP/Contents/MacOS"

if [ "$PROFILE" = "release" ]; then
  cargo build --locked --release --workspace
  TARGET=target/release
else
  cargo build --locked --workspace
  TARGET=target/debug
fi

rm -rf "$APP"
mkdir -p "$MACOS"
cp "$TARGET/pinchd" "$TARGET/pinch-ui" "$TARGET/pinch" "$MACOS/"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>sh.pinch.Pinch</string>
  <key>CFBundleName</key><string>Pinch</string>
  <key>CFBundleDisplayName</key><string>Pinch</string>
  <key>CFBundleExecutable</key><string>pinchd</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Ad-hoc signature: enough for local use. Distributing to other people needs a
# Developer ID certificate and notarization.
codesign --force --deep --sign - "$APP"

echo
echo "Built $APP ($PROFILE)."
echo "  Start:   open $APP        (menu bar icon appears)"
echo "  CLI:     $MACOS/pinch --help"
echo "  Install: ln -sf \"\$PWD/$MACOS/pinch\" ~/.local/bin/pinch"
