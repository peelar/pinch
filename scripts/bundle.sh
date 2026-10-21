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
VARIANT=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --variant)
      [ "$#" -ge 2 ] || { echo '--variant needs a name' >&2; exit 1; }
      VARIANT="$2"
      [[ "$VARIANT" =~ ^[a-z][a-z0-9]*(-[a-z0-9]+)*$ ]] && [ "${#VARIANT}" -le 32 ] || { echo 'Variant must be a lowercase slug of at most 32 characters' >&2; exit 1; }
      shift 2;;
    --help) echo 'Usage: bundle.sh [--variant proactive]'; exit 0;;
    *) echo "Unknown option: $1" >&2; exit 1;;
  esac
done
APP_NAME=Pinch
BUNDLE_ID=sh.pinch.Pinch
if [ -n "$VARIANT" ]; then
  LABEL=$(printf '%s' "$VARIANT" | awk -F- '{for (i=1;i<=NF;i++) printf "%s%s%s", (i>1?" ":""), toupper(substr($i,1,1)), substr($i,2)}')
  APP_NAME="Pinch $LABEL"
  BUNDLE_ID="$BUNDLE_ID.$VARIANT"
fi
VERSION=$(sed -n '/\[workspace.package\]/,/^\[/s/^version = "\([^"]*\)"/\1/p' Cargo.toml)
APP="dist/$APP_NAME.app"
MACOS="$APP/Contents/MacOS"

if [ "$PROFILE" = "release" ]; then
  cargo build --locked --release --workspace
  TARGET=target/release
else
  cargo build --locked --workspace
  TARGET=target/debug
fi

rm -rf "$APP"
mkdir -p "$MACOS" "$APP/Contents/Resources"
if [ -n "$VARIANT" ]; then printf '%s\n' "$VARIANT" > "$APP/Contents/Resources/pinch-variant"; fi
cp assets/brand/Pinch.icns "$APP/Contents/Resources/Pinch.icns"
cp crates/ui/assets/icons/LICENSE "$APP/Contents/Resources/Zeron-Icons-LICENSE"
cp "$TARGET/pinchd" "$TARGET/pinch-ui" "$TARGET/pinch" "$MACOS/"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleName</key><string>$APP_NAME</string>
  <key>CFBundleDisplayName</key><string>$APP_NAME</string>
  <key>CFBundleIconFile</key><string>Pinch</string>
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

# Variants must never update into the public application identity.
if [ -z "$VARIANT" ] || [ -n "${PINCH_UPDATE_FEED_URL:-}" ]; then
  SPARKLE=$(scripts/fetch-sparkle.sh)
  mkdir -p "$APP/Contents/Frameworks"
  ditto "$SPARKLE/Sparkle.framework" "$APP/Contents/Frameworks/Sparkle.framework"
  cp "$SPARKLE/LICENSE" "$APP/Contents/Resources/Sparkle-LICENSE"
  clang -dynamiclib -fobjc-arc -mmacosx-version-min=13.0 \
    -F "$SPARKLE" -framework Sparkle -framework AppKit \
    -Wl,-rpath,@loader_path -Wl,-install_name,@rpath/libpinch-updater.dylib \
    native/updater/bridge.m -o "$APP/Contents/Frameworks/libpinch-updater.dylib"
  PLIST="$APP/Contents/Info.plist"
  FEED_URL="${PINCH_UPDATE_FEED_URL:-https://github.com/peelar/pinch/releases/latest/download/appcast.xml}"
  /usr/libexec/PlistBuddy -c "Add :SUFeedURL string $FEED_URL" "$PLIST"
  /usr/libexec/PlistBuddy -c "Add :SUPublicEDKey string $(cat native/updater/public-key.txt)" "$PLIST"
  /usr/libexec/PlistBuddy -c 'Add :SUEnableAutomaticChecks bool true' "$PLIST"
  /usr/libexec/PlistBuddy -c 'Add :SUAutomaticallyUpdate bool false' "$PLIST"
  /usr/libexec/PlistBuddy -c 'Add :SUAllowsAutomaticUpdates bool false' "$PLIST"
fi

# Ad-hoc signature: enough for local use. Distributing to other people needs a
# Developer ID certificate and notarization.
codesign --force --deep --sign - "$APP"

echo
echo "Built $APP ($PROFILE)."
echo "  Start:   open \"$APP\"        (menu bar icon appears)"
echo "  CLI:     \"$MACOS/pinch\" --help"
echo "  Install: scripts/install.sh --app \"\$PWD/$APP\" --no-modify-path"
