#!/usr/bin/env bash
# Direct macOS installer. Release packaging replaces the repository marker.
set -euo pipefail

REPOSITORY="${PINCH_REPOSITORY:-__PINCH_REPOSITORY__}"
VERSION=latest
LOCAL_APP=
MODIFY_PATH=1
APP_DIR="${PINCH_APP_DIR:-$HOME/Applications}"
BIN_DIR="${PINCH_BIN_DIR:-$HOME/.local/bin}"
fail() { echo "pinch: $*" >&2; exit 1; }
while [ "$#" -gt 0 ]; do
  case "$1" in
    --repo|--version|--app)
      [ "$#" -ge 2 ] || fail "$1 needs a value"
      case "$1" in --repo) REPOSITORY="$2";; --version) VERSION="$2";; --app) LOCAL_APP="$2";; esac
      shift 2;;
    --no-modify-path) MODIFY_PATH=0; shift;;
    --help) echo 'Usage: install.sh [--repo owner/repo] [--version v0.1.0] [--app /path/Pinch.app] [--no-modify-path]'; exit 0;;
    *) fail "unknown option: $1";;
  esac
done
[ "$(uname -s)" = Darwin ] || fail 'Pinch currently supports macOS only'
case "$(uname -m)" in arm64) ARCH=arm64;; x86_64) ARCH=x86_64;; *) fail 'unsupported architecture';; esac
[ "$(sw_vers -productVersion | cut -d. -f1)" -ge 13 ] || fail 'macOS 13 or newer is required'
mkdir -p "$APP_DIR" "$BIN_DIR"
[ ! -e "$BIN_DIR/pinch" ] || [ -L "$BIN_DIR/pinch" ] || fail "$BIN_DIR/pinch already exists and is not a symlink"
WORK=$(mktemp -d "$APP_DIR/.pinch-install.XXXXXX")
trap 'rm -rf "$WORK"' EXIT
if [ -n "$LOCAL_APP" ]; then
  ditto "$LOCAL_APP" "$WORK/Pinch.app"
else
  [[ "$REPOSITORY" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || fail 'set --repo owner/repo (no public repository is configured yet)'
  [[ "$VERSION" =~ ^[A-Za-z0-9_.-]+$ ]] || fail 'invalid version'
  BASE="https://github.com/$REPOSITORY/releases"
  if [ "$VERSION" = latest ]; then BASE="$BASE/latest/download"; else BASE="$BASE/download/$VERSION"; fi
  ASSET="pinch-macos-$ARCH.zip"
  curl --fail --location --silent --show-error "$BASE/$ASSET" -o "$WORK/$ASSET"
  curl --fail --location --silent --show-error "$BASE/$ASSET.sha256" -o "$WORK/$ASSET.sha256"
  (cd "$WORK" && shasum -a 256 -c "$ASSET.sha256")
  ditto -x -k "$WORK/$ASSET" "$WORK"
  # Ad hoc signatures check bundle integrity, without asserting a developer identity.
  codesign --verify --deep --strict "$WORK/Pinch.app"
  if ! spctl --assess --type execute "$WORK/Pinch.app" >/dev/null 2>&1; then
    echo "This release is not notarized by Apple. macOS may ask you to approve Pinch."
  fi
fi
for binary in pinch pinchd pinch-ui; do
  [ -x "$WORK/Pinch.app/Contents/MacOS/$binary" ] || fail "bundle is missing $binary"
done
# Stop the old engine so the next CLI invocation uses the updated binaries.
# The quit command leaves managed servers running.
if [ -x "$APP_DIR/Pinch.app/Contents/MacOS/pinch" ]; then
  "$APP_DIR/Pinch.app/Contents/MacOS/pinch" --json quit
fi
if [ -e "$APP_DIR/Pinch.app" ]; then mv "$APP_DIR/Pinch.app" "$WORK/previous.app"; fi
if ! mv "$WORK/Pinch.app" "$APP_DIR/Pinch.app"; then
  [ ! -e "$WORK/previous.app" ] || mv "$WORK/previous.app" "$APP_DIR/Pinch.app"
  fail 'could not install the app'
fi
ln -sfn "$APP_DIR/Pinch.app/Contents/MacOS/pinch" "$BIN_DIR/pinch"
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *)
    CONFIG=
    case "${SHELL:-}" in */zsh) CONFIG="$HOME/.zshrc";; */bash) CONFIG="$HOME/.bash_profile";; esac
    if [ "$MODIFY_PATH" = 1 ] && [ -n "$CONFIG" ]; then
      printf -v PATH_LINE 'export PATH=%q:"$PATH"' "$BIN_DIR"
      if ! [ -f "$CONFIG" ] || ! grep -Fqx "$PATH_LINE" "$CONFIG"; then
        printf '\n# Pinch command-line tool\n%s\n' "$PATH_LINE" >> "$CONFIG"
      fi
      echo "Added Pinch to $CONFIG. Open a new terminal to use pinch."
    else
      printf 'Add this directory to PATH: %s\n' "$BIN_DIR"
    fi;;
esac
echo "Installed $APP_DIR/Pinch.app"
if ! "$APP_DIR/Pinch.app/Contents/MacOS/pinch" --version; then
  echo 'macOS could not launch Pinch. Open Pinch.app, then approve it in System Settings > Privacy & Security > Open Anyway if prompted.'
  echo 'See https://support.apple.com/102445 for macOS approval instructions.'
fi
echo 'Next: pinch setup, then pinch ps. Rerun this installer to update.'
