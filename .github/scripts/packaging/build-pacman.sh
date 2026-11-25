#!/usr/bin/env bash
#
# Build podup-<V>-1-x86_64.pkg.tar.zst from a staged payload.
#
#   build-pacman.sh STAGE_DIR OUT_DIR VERSION
#
#   STAGE_DIR  written by stage-payload.py for amd64 (root/ and files.txt)
#   OUT_DIR    receives the one package
#   VERSION    X.Y.Z, the tag without `v`
#
# There is no aarch64 package: Arch Linux has no official port, and the
# channel refuses any architecture but x86_64 and any. Nothing is compiled. The
# tools (bsdtar, zstd) are the ones in the pinned Arch image (images.env), which
# runs with no network.
#
# Env: SOURCE_DATE_EPOCH (required), CONTAINER_ENGINE (podman, or docker).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
stage="${1:?stage dir}"
out="${2:?output dir}"
version="${3:?version}"
: "${SOURCE_DATE_EPOCH:?set SOURCE_DATE_EPOCH to the release commit time}"
engine="${CONTAINER_ENGINE:-podman}"

[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "build-pacman: version '$version' is not X.Y.Z" >&2; exit 1; }
[[ "$SOURCE_DATE_EPOCH" =~ ^[0-9]{1,12}$ ]] || { echo "build-pacman: SOURCE_DATE_EPOCH is not a number" >&2; exit 1; }
if [ ! -d "$stage/root" ] || [ ! -f "$stage/files.txt" ]; then
	echo "build-pacman: $stage is not a staged payload" >&2
	exit 1
fi
# Fail on a payload that is not the x86_64 one before packing anything.
if ! head -c 20 "$stage/root/usr/bin/podup" | od -An -tx1 | tr -d ' \n' | grep -q '^7f454c46.\{28\}3e00'; then
	echo "build-pacman: usr/bin/podup is not an x86_64 ELF" >&2
	exit 1
fi
mkdir -p "$out"
stage="$(cd "$stage" && pwd)"
out="$(cd "$out" && pwd)"

# shellcheck disable=SC1091 # a sibling file, not followed by the lint
. "$here/images.env"

name="podup-${version}-1-x86_64.pkg.tar.zst"
rm -f "$out/$name"
"$engine" run --rm --network none \
	--env "VERSION=$version" --env "SOURCE_DATE_EPOCH=$SOURCE_DATE_EPOCH" \
	--volume "$stage:/stage:ro" \
	--volume "$here/pacman/inner.sh:/work/inner.sh:ro" \
	--volume "$out:/out" \
	"$PACMAN_BUILD_BASE" bash /work/inner.sh
[ -f "$out/$name" ] || { echo "build-pacman: $name was not produced" >&2; exit 1; }
echo "$out/$name"
