#!/usr/bin/env bash
#
# Build the signed podup apt repository into a directory ready to publish as a
# static site (GitHub Pages). A fresh repository is generated on every run — the
# project ships no old-version support, so only the current release is carried.
#
# Requires: reprepro, gpg.
# Reads the armored private signing key from $PODUP_APT_GPG_PRIVATE_KEY.
#
# Usage:
#   PODUP_APT_GPG_PRIVATE_KEY="$(cat priv.asc)" \
#     build-repo.sh <output-dir> <deb> [<deb> ...]

set -euo pipefail

OUT_DIR="${1:?usage: build-repo.sh <output-dir> <deb> [<deb> ...]}"
shift
[ "$#" -ge 1 ] || { echo "no .deb files given" >&2; exit 1; }

: "${PODUP_APT_GPG_PRIVATE_KEY:?PODUP_APT_GPG_PRIVATE_KEY is not set}"

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PUBKEY_ASC="$HERE/podup-apt-key.asc"
[ -f "$PUBKEY_ASC" ] || { echo "missing public key: $PUBKEY_ASC" >&2; exit 1; }

DOMAIN="apt.glyndor.net"

# Resolve .deb paths before we change directories.
DEBS=()
for d in "$@"; do
	[ -f "$d" ] || { echo "no such .deb: $d" >&2; exit 1; }
	DEBS+=("$(cd "$(dirname "$d")" && pwd)/$(basename "$d")")
done

mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"

# Isolated keyring; never touches the caller's gpg state.
GNUPGHOME="$(mktemp -d)"
export GNUPGHOME
chmod 700 "$GNUPGHOME"
cleanup() { gpgconf --kill all 2>/dev/null || true; rm -rf "$GNUPGHOME"; }
trap cleanup EXIT

printf '%s' "$PODUP_APT_GPG_PRIVATE_KEY" | gpg --batch --quiet --import 2>/dev/null
FPR="$(gpg --batch --with-colons --list-secret-keys | awk -F: '/^fpr:/{print $10; exit}')"
[ -n "$FPR" ] || { echo "could not determine signing key fingerprint" >&2; exit 1; }

# Fail closed if the committed public key does not match the private signing key
# in the secret — a mismatch would publish a repo no installed keyring can verify.
PUB_FPR="$(gpg --batch --with-colons --show-keys "$PUBKEY_ASC" | awk -F: '/^fpr:/{print $10; exit}')"
if [ "$PUB_FPR" != "$FPR" ]; then
	echo "::error::committed public key ($PUB_FPR) does not match signing key ($FPR)" >&2
	exit 1
fi

# reprepro repository skeleton.
CONF="$OUT_DIR/conf"
mkdir -p "$CONF"
cat > "$CONF/distributions" <<EOF
Origin: Glyndor
Label: podup
Suite: stable
Codename: stable
Architectures: amd64
Components: main
Description: podup apt repository
SignWith: $FPR
EOF

reprepro -b "$OUT_DIR" includedeb stable "${DEBS[@]}"

# reprepro's bookkeeping must not be served publicly.
rm -rf "$OUT_DIR/conf" "$OUT_DIR/db"

# Static-site extras.
touch "$OUT_DIR/.nojekyll"
printf '%s\n' "$DOMAIN" > "$OUT_DIR/CNAME"
# Offer the armored key for manual setups (deb822 Signed-By still uses the deb).
cp "$PUBKEY_ASC" "$OUT_DIR/podup-apt-key.asc"

cat > "$OUT_DIR/index.html" <<EOF
<!doctype html>
<meta charset="utf-8">
<title>podup apt repository</title>
<h1>podup apt repository</h1>
<p>Install on Debian/Ubuntu (amd64):</p>
<pre>curl -fsSL https://glyndor.net/install/podup | bash -s -- --apt</pre>
<p>Source: <a href="https://github.com/Glyndor/podup">github.com/Glyndor/podup</a></p>
EOF

echo "apt repository built at $OUT_DIR (signed by $FPR)"
