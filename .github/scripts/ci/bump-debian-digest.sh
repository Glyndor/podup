#!/usr/bin/env bash
#
# Usage: bump-debian-digest.sh [sha256:<64 hex>]
# Run from the root of the repository. Raises the digest of the `debian:trixie` image that the release builds
# in and that the debian-build lane exercises, in the two files that pin it, to the one given or, with no
# argument, to the one the registry answers for the `trixie` tag today.
#
# This is the bump path of that digest. Dependabot cannot take it: the `github-actions` ecosystem does not
# update a workflow's `container:` image, and the weekly pin watch (pin-watch.yml) only says when it has drifted.
# The two have to move together (the lane is meant to run in the image the release ships from) and a test holds
# that they do, so a bump that moves one is refused: each file has to hold exactly one pin and both are written
# or neither is.
#
# It changes the digests and nothing else, prints what it changed, and does not commit.
set -euo pipefail
export LC_ALL=C

release=.github/workflows/reusable-release-build.yml
lane=.github/workflows/reusable-rust-debian.yml
pin='debian:trixie@sha256:[a-f0-9]{64}'

new="${1:-}"
if [ -z "$new" ]; then
	token="$(curl -fsSL 'https://auth.docker.io/token?service=registry.docker.io&scope=repository:library/debian:pull' |
		python3 -c 'import sys,json; print(json.load(sys.stdin)["token"])')"
	new="$(curl -fsSL -I -H "Authorization: Bearer $token" \
		-H 'Accept: application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json' \
		'https://registry-1.docker.io/v2/library/debian/manifests/trixie' |
		tr -d '\r' | awk -F': ' 'tolower($1) == "docker-content-digest" { print $2; exit }')"
fi
if ! [[ "$new" =~ ^sha256:[a-f0-9]{64}$ ]]; then
	echo "bump-debian-digest: '$new' is not a sha256 digest" >&2
	exit 2
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
for f in "$release" "$lane"; do
	[ -f "$f" ] || { echo "bump-debian-digest: $f is not here; run it from the root of the repository" >&2; exit 2; }
	n="$(grep -cE "$pin" "$f" || true)"
	if [ "$n" -ne 1 ]; then
		echo "bump-debian-digest: $f holds $n pins of debian:trixie, not exactly one; nothing is changed" >&2
		exit 2
	fi
	old="$(grep -oE "$pin" "$f" | sed 's/.*@//')"
	sed -E "s#debian:trixie@sha256:[a-f0-9]{64}#debian:trixie@${new}#" "$f" >"$tmp/$(basename "$f")"
	echo "$f: $old -> $new"
done
for f in "$release" "$lane"; do
	cp "$tmp/$(basename "$f")" "$f"
done
