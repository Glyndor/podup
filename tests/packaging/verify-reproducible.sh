#!/usr/bin/env bash
#
# The same input gives the same bytes, and the epoch is what makes it so.
#
#   verify-reproducible.sh STAGE_AMD64 STAGE_ARM64 VERSION
#
# Builds each package twice, apart in time and under another time zone, and
# requires equal checksums; then once more with another SOURCE_DATE_EPOCH and
# requires different ones, so a build that ignored the variable would not pass
# by being constant.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
bin="$here/../../.github/scripts/packaging"
amd64="${1:?amd64 stage}"
arm64="${2:?arm64 stage}"
version="${3:?version}"
epoch=1795380000

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

sums() { # label epoch tz
	local label="$1" at="$2" tz="$3"
	(
		export SOURCE_DATE_EPOCH="$at" TZ="$tz"
		"$bin/build-rpm.sh" "$amd64" "$tmp/$label/rpm-x86" "$version" x86_64 >/dev/null
		"$bin/build-rpm.sh" "$arm64" "$tmp/$label/rpm-arm" "$version" aarch64 >/dev/null
		"$bin/build-pacman.sh" "$amd64" "$tmp/$label/pacman" "$version" >/dev/null
	)
	(cd "$tmp/$label" && find . -type f -print0 | LC_ALL=C sort -z | xargs -0 sha256sum)
}

first="$(sums one "$epoch" UTC)"
sleep 2
second="$(sums two "$epoch" Asia/Tokyo)"
[ "$first" = "$second" ] || { echo "verify-reproducible: two builds of the same input differ:" >&2; diff <(echo "$first") <(echo "$second") >&2 || true; exit 1; }
echo "ok    the same input gives the same bytes (3 packages)"

third="$(sums three $((epoch + 1)) UTC)"
if [ "$(echo "$first" | cut -d' ' -f1)" = "$(echo "$third" | cut -d' ' -f1)" ]; then
	echo "verify-reproducible: another SOURCE_DATE_EPOCH gave the same bytes" >&2
	exit 1
fi
echo "ok    another SOURCE_DATE_EPOCH gives other bytes"
