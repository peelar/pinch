#!/usr/bin/env bash
# Direct macOS installer. Release packaging replaces the repository marker.
set -euo pipefail

REPOSITORY="${PINCH_REPOSITORY:-__PINCH_REPOSITORY__}"
VERSION=latest
LOCAL_APP=
EXPECTED_VARIANT=
MODIFY_PATH=1
APP_DIR="${PINCH_APP_DIR:-$HOME/Applications}"
BIN_DIR="${PINCH_BIN_DIR:-$HOME/.local/bin}"
fail() { echo "pinch: $*" >&2; exit 1; }
while [ "$#" -gt 0 ]; do
  case "$1" in
    --repo|--version|--app|--variant)
      [ "$#" -ge 2 ] || fail "$1 needs a value"
      case "$1" in --repo) REPOSITORY="$2";; --version) VERSION="$2";; --app) LOCAL_APP="$2";; --variant)
        EXPECTED_VARIANT="$2"
        [[ "$EXPECTED_VARIANT" =~ ^[a-z][a-z0-9]*(-[a-z0-9]+)*$ ]] && [ "${#EXPECTED_VARIANT}" -le 32 ] || fail 'variant must be a lowercase slug of at most 32 characters';; esac
      shift 2;;
    --no-modify-path) MODIFY_PATH=0; shift;;
    --help) echo 'Usage: install.sh [--repo owner/repo] [--version v0.1.0] [--app /path/Pinch.app] [--variant proactive] [--no-modify-path]'; exit 0;;
    *) fail "unknown option: $1";;
  esac
done
[ "$(uname -s)" = Darwin ] || fail 'Pinch currently supports macOS only'
case "$(uname -m)" in arm64) ARCH=arm64;; x86_64) ARCH=x86_64;; *) fail 'unsupported architecture';; esac
[ "$(sw_vers -productVersion | cut -d. -f1)" -ge 13 ] || fail 'macOS 13 or newer is required'
mkdir -p "$APP_DIR" "$BIN_DIR"
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
VARIANT=
if [ -e "$WORK/Pinch.app/Contents/Resources/pinch-variant" ]; then
  VARIANT=$(cat "$WORK/Pinch.app/Contents/Resources/pinch-variant")
  [[ "$VARIANT" =~ ^[a-z][a-z0-9]*(-[a-z0-9]+)*$ ]] && [ "${#VARIANT}" -le 32 ] || fail 'bundle contains an invalid variant'
fi
[ -z "$EXPECTED_VARIANT" ] || [ "$EXPECTED_VARIANT" = "$VARIANT" ] || fail 'requested variant does not match the bundle; build it with bundle.sh --variant first'
APP_NAME=Pinch
CLI_NAME=pinch
BUNDLE_ID=sh.pinch.Pinch
if [ -n "$VARIANT" ]; then
  LABEL=$(printf '%s' "$VARIANT" | awk -F- '{for (i=1;i<=NF;i++) printf "%s%s%s", (i>1?" ":""), toupper(substr($i,1,1)), substr($i,2)}')
  APP_NAME="Pinch $LABEL"
  CLI_NAME="pinch-$VARIANT"
  BUNDLE_ID="$BUNDLE_ID.$VARIANT"
fi
ACTUAL_ID=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$WORK/Pinch.app/Contents/Info.plist")
[ "$ACTUAL_ID" = "$BUNDLE_ID" ] || fail 'bundle identifier does not match its variant'
DEST="$APP_DIR/$APP_NAME.app"
[ ! -e "$BIN_DIR/$CLI_NAME" ] || [ -L "$BIN_DIR/$CLI_NAME" ] || fail "$BIN_DIR/$CLI_NAME already exists and is not a symlink"
# Quit only the matching installation, using its own default state directory.
# PINCH_HOME in the caller's shell must not redirect an update to another engine.
if [ -e "$DEST" ]; then
  INSTALLED_ID=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$DEST/Contents/Info.plist")
  [ "$INSTALLED_ID" = "$BUNDLE_ID" ] || fail 'destination belongs to another app'
  INSTALLED_VARIANT=
  if [ -e "$DEST/Contents/Resources/pinch-variant" ]; then
    INSTALLED_VARIANT=$(cat "$DEST/Contents/Resources/pinch-variant")
  fi
  [ "$INSTALLED_VARIANT" = "$VARIANT" ] || fail 'destination belongs to another variant'
fi
env -u PINCH_HOME "$WORK/Pinch.app/Contents/MacOS/pinch" --json quit
if [ -e "$DEST" ]; then mv "$DEST" "$WORK/previous.app"; fi
if ! mv "$WORK/Pinch.app" "$DEST"; then
  [ ! -e "$WORK/previous.app" ] || mv "$WORK/previous.app" "$DEST"
  fail 'could not install the app'
fi
ln -sfn "$DEST/Contents/MacOS/pinch" "$BIN_DIR/$CLI_NAME"
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
echo "Installed $DEST"
if ! "$DEST/Contents/MacOS/pinch" --version; then
  echo 'macOS could not launch Pinch. Open Pinch.app, then approve it in System Settings > Privacy & Security > Open Anyway if prompted.'
  echo 'See https://support.apple.com/102445 for macOS approval instructions.'
fi
echo "Next: $CLI_NAME ps, or open \"$DEST\". Rerun this installer to update."
