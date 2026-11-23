#!/bin/bash
# Install ebb-strap, Ebb's Bluetooth helper (source in helper/).
#
# Downloads the release binary for this machine from this repository's GitHub
# release and installs it only if its SHA-256 matches the value pinned below.
# Nothing downloaded runs before that check. Opened in a terminal by the
# panel's "Install helper" button; safe to run by hand.

set -euo pipefail

VERSION=1.5.0
declare -A SHA256=(
  [x86_64]=7c3469233d11ab4627c4b87bad12bdff205dbe481ac711e3359dc4701465452e
  [aarch64]=aec13102f7979f8d4f2a86c3e930f2d0b2c7be02b5cd3351f5004b39e7fa5198
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
