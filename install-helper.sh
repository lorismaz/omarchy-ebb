#!/bin/bash
# Install ebb-strap, Ebb's Bluetooth helper (source in helper/).
#
# Downloads the release binary for this machine from this repository's GitHub
# release and installs it only if its SHA-256 matches the value pinned below.
# Nothing downloaded runs before that check. Opened in a terminal by the
# panel's "Install helper" button; safe to run by hand.

set -euo pipefail

VERSION=1.3.0
declare -A SHA256=(
  [x86_64]=30e5546b4e95455b16e9421005aed6b3d208e90a5e436ce01fbbc7f6c9a3fe86
  [aarch64]=101704b5fcfa083c27940f5ca78a379b30356e3a9acd818a7a6d84f16e4f4c24
)
REPO=https://github.com/lorismaz/omarchy-ebb
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
DEST="$DATA/ebb/bin"

if command -v ebb-strap >/dev/null; then
  echo "ebb-strap is already installed: $(command -v ebb-strap)"
  exit 0
fi

arch=$(uname -m)
expected=${SHA256[$arch]:-}
if [[ ! $expected =~ ^[0-9a-f]{64}$ ]]; then
  echo "There is no ebb-strap $VERSION build for $arch yet."
  exit 1
fi

name="ebb-strap-$VERSION-$arch-linux"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "Downloading ebb-strap $VERSION for $arch…"
curl -fL --progress-bar --proto '=https' --proto-redir '=https' --tlsv1.2 --max-time 120 --max-filesize 50000000 \
  -o "$tmp/$name" "$REPO/releases/download/v$VERSION/$name"

actual=$(sha256sum "$tmp/$name" | cut -d' ' -f1)
if [[ $actual != "$expected" ]]; then
  echo "The download does not match the pinned checksum, so it was not installed."
  echo "  expected $expected"
  echo "  got      $actual"
  exit 1
fi

install -Dm755 "$tmp/$name" "$DEST/ebb-strap"
# Ebb was called Helio Strap: its old helper is no longer used.
rm -rf "$DATA/helio-strap"
echo "Installed $("$DEST/ebb-strap" --version) to $DEST."
echo "The panel picks it up within a few seconds."
