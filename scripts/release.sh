#!/usr/bin/env bash
# Package an existing bundle, optionally signing and notarizing it first.
set -euo pipefail
cd "$(dirname "$0")/.."
APP=dist/Pinch.app
ARCH="${ARCH:-$(uname -m)}"
case "$ARCH" in arm64|x86_64) ;; *) echo 'Unsupported architecture' >&2; exit 1;; esac
if [ -n "${SIGNING_IDENTITY:-}" ]; then
  : "${NOTARY_PROFILE:?Set NOTARY_PROFILE to a notarytool keychain profile}"
  for binary in pinch pinchd pinch-ui; do
    codesign --force --options runtime --timestamp --sign "$SIGNING_IDENTITY" "$APP/Contents/MacOS/$binary"
  done
  codesign --force --options runtime --timestamp --sign "$SIGNING_IDENTITY" "$APP"
  ditto -c -k --keepParent "$APP" dist/notarize.zip
  xcrun notarytool submit dist/notarize.zip --keychain-profile "$NOTARY_PROFILE" --wait
  xcrun stapler staple "$APP"
  rm dist/notarize.zip
  spctl --assess --type execute "$APP"
else
  echo 'Packaging a local, ad hoc signed build; public releases need Developer ID signing and notarization.' >&2
fi
codesign --verify --deep --strict "$APP"
ASSET="pinch-macos-$ARCH.zip"
rm -f "dist/$ASSET"
ditto -c -k --keepParent "$APP" "dist/$ASSET"
(cd dist && shasum -a 256 "$ASSET" > "$ASSET.sha256")
if [ -n "${PINCH_REPOSITORY:-}" ]; then
  [[ "$PINCH_REPOSITORY" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || exit 1
  sed "s|__PINCH_REPOSITORY__|$PINCH_REPOSITORY|g" scripts/install.sh > dist/install.sh
else
  cp scripts/install.sh dist/install.sh
fi
chmod +x dist/install.sh
echo "Packaged dist/$ASSET"
