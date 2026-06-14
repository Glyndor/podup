#!/usr/bin/env bash
#
# Build the podup-archive-keyring .deb.
#
# This package installs the apt repository's signing key and source list so a
# user can `apt install podup` and receive updates — including key renewals —
# through the normal `apt upgrade` flow. apt owns the key file, so a renewed key
# propagates automatically; nothing for the user to re-run.
#
# Usage:
#   build-keyring.sh <version> <output-dir>
#
# Produces: <output-dir>/podup-archive-keyring.deb (version lives in the control
# file, not the filename, so the release asset has a stable latest/download URL).

set -euo pipefail

VERSION="${1:?usage: build-keyring.sh <version> <output-dir>}"
OUT_DIR="${2:?usage: build-keyring.sh <version> <output-dir>}"

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PUBKEY_ASC="$HERE/podup-apt-key.asc"
SOURCES="$HERE/podup.sources"

[ -f "$PUBKEY_ASC" ] || { echo "missing public key: $PUBKEY_ASC" >&2; exit 1; }
[ -f "$SOURCES" ]    || { echo "missing sources file: $SOURCES" >&2; exit 1; }

mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"

ROOT="$(mktemp -d)"
trap 'rm -rf "$ROOT"' EXIT
chmod 0755 "$ROOT"

install -d -m 0755 "$ROOT/DEBIAN"
install -d -m 0755 "$ROOT/usr/share/keyrings"
install -d -m 0755 "$ROOT/etc/apt/sources.list.d"

# apt needs the key dearmored (binary OpenPGP), referenced by Signed-By.
gpg --dearmor < "$PUBKEY_ASC" > "$ROOT/usr/share/keyrings/podup.gpg"
chmod 0644 "$ROOT/usr/share/keyrings/podup.gpg"

install -m 0644 "$SOURCES" "$ROOT/etc/apt/sources.list.d/podup.sources"

cat > "$ROOT/DEBIAN/control" <<EOF
Package: podup-archive-keyring
Version: $VERSION
Architecture: all
Maintainer: Glyndor <75870284+Jaro-c@users.noreply.github.com>
Section: utils
Priority: optional
Homepage: https://github.com/Glyndor/podup
Description: GPG key and apt source for the podup repository
 Installs the signing key and source list for the podup apt repository at
 https://apt.glyndor.net so that podup can be installed and kept up to date
 with apt. Key renewals are delivered through apt upgrade.
EOF

# Mark the source list as a conffile so a local edit is preserved across upgrades.
cat > "$ROOT/DEBIAN/conffiles" <<'EOF'
/etc/apt/sources.list.d/podup.sources
EOF

dpkg-deb --root-owner-group --build "$ROOT" \
	"$OUT_DIR/podup-archive-keyring.deb" >/dev/null

echo "$OUT_DIR/podup-archive-keyring.deb"
