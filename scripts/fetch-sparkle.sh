#!/usr/bin/env bash
# Pinned upstream binary distribution, shared by bundling and release tooling.
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION=2.10.0
SHA256=c2bf58aa8387266ac179357b1415d6f2635f044da8be41042af32425dae6da0c
DEST="$PWD/target/sparkle/$VERSION"
if [ ! -f "$DEST/.verified" ]; then
  mkdir -p target/sparkle
  ARCHIVE="$PWD/target/sparkle/Sparkle-$VERSION.tar.xz"
  curl --fail --location --silent --show-error "https://github.com/sparkle-project/Sparkle/releases/download/$VERSION/Sparkle-$VERSION.tar.xz" -o "$ARCHIVE"
  printf '%s  %s\n' "$SHA256" "$ARCHIVE" | shasum -a 256 -c - >&2
  rm -rf "$DEST"
  mkdir -p "$DEST"
  tar -xf "$ARCHIVE" -C "$DEST"
  touch "$DEST/.verified"
fi
printf '%s\n' "$DEST"
