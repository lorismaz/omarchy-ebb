#!/bin/bash
# Install helio-strap, the plugin's Bluetooth helper (source in helper/).
#
# Downloads the release binary for this machine from this repository's GitHub
# release and installs it only if its SHA-256 matches the value pinned below.
# Nothing downloaded runs before that check. Opened in a terminal by the
# panel's "Install helper" button; safe to run by hand.

set -euo pipefail

VERSION=1.2.0
declare -A SHA256=(
  [x86_64]=b6777a065b51547ea6d6f12f073f751bce23f76f1c7fda04fbfefd2d3d3bbd09
  [aarch64]=0461270c5c583c707b427d2f2b94a2e356344ee2b9186c04b8edb1f09c6b8f3f
)
REPO=https://github.com/lorismaz/omarchy-helio-strap
DEST="${XDG_DATA_HOME:-$HOME/.local/share}/helio-strap/bin"

if command -v helio-strap >/dev/null; then
  echo "helio-strap is already installed: $(command -v helio-strap)"
  exit 0
fi

arch=$(uname -m)
expected=${SHA256[$arch]:-}
if [[ ! $expected =~ ^[0-9a-f]{64}$ ]]; then
  echo "There is no helio-strap $VERSION build for $arch yet."
  exit 1
fi

name="helio-strap-$VERSION-$arch-linux"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "Downloading helio-strap $VERSION for $arch…"
curl -fL --progress-bar --proto '=https' --proto-redir '=https' --tlsv1.2 --max-time 120 --max-filesize 50000000 \
  -o "$tmp/$name" "$REPO/releases/download/v$VERSION/$name"

actual=$(sha256sum "$tmp/$name" | cut -d' ' -f1)
if [[ $actual != "$expected" ]]; then
  echo "The download does not match the pinned checksum, so it was not installed."
  echo "  expected $expected"
  echo "  got      $actual"
  exit 1
fi

install -Dm755 "$tmp/$name" "$DEST/helio-strap"
echo "Installed $("$DEST/helio-strap" --version) to $DEST."
echo "The panel picks it up within a few seconds."
