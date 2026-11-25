#!/usr/bin/env bash
#
# Build podup-<V>-1.glyndor.any.<arch>.rpm from a staged payload.
#
#   build-rpm.sh STAGE_DIR OUT_DIR VERSION RPM_ARCH
#
#   STAGE_DIR  written by stage-payload.py (root/ and files.txt)
#   OUT_DIR    receives the one .rpm
#   VERSION    X.Y.Z, the tag without `v`
#   RPM_ARCH   x86_64 or aarch64
#
# Nothing is compiled. rpmbuild runs in a container built from the pinned
# AlmaLinux 9 image (images.env) with no network, so the package depends on the
# staged files, the version, the architecture and SOURCE_DATE_EPOCH, and on
# nothing else: the same input gives the same bytes.
#
# Env: SOURCE_DATE_EPOCH (required), CONTAINER_ENGINE (podman, or docker).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
stage="${1:?stage dir}"
out="${2:?output dir}"
version="${3:?version}"
arch="${4:?x86_64 or aarch64}"
: "${SOURCE_DATE_EPOCH:?set SOURCE_DATE_EPOCH to the release commit time}"
engine="${CONTAINER_ENGINE:-podman}"

[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "build-rpm: version '$version' is not X.Y.Z" >&2; exit 1; }
case "$arch" in x86_64 | aarch64) ;; *) echo "build-rpm: architecture '$arch' is not x86_64 or aarch64" >&2; exit 1 ;; esac
[[ "$SOURCE_DATE_EPOCH" =~ ^[0-9]{1,12}$ ]] || { echo "build-rpm: SOURCE_DATE_EPOCH is not a number" >&2; exit 1; }
if [ ! -d "$stage/root" ] || [ ! -f "$stage/files.txt" ]; then
	echo "build-rpm: $stage is not a staged payload" >&2
	exit 1
fi
mkdir -p "$out"
stage="$(cd "$stage" && pwd)"
out="$(cd "$out" && pwd)"

# shellcheck disable=SC1091 # a sibling file, not followed by the lint
. "$here/images.env"

# The image is named by what it is made from, so a change to either file builds
# a new one and an unchanged pair reuses it.
tag="localhost/podup-rpmbuild:$(cat "$here/images.env" "$here/rpm/Containerfile" | sha256sum | cut -c1-16)"
if ! "$engine" image inspect "$tag" >/dev/null 2>&1; then
	"$engine" build --quiet --tag "$tag" --build-arg "BASE=$RPM_BUILD_BASE" \
		--file "$here/rpm/Containerfile" "$here/rpm" >/dev/null
fi

name="podup-${version}-1.glyndor.any.${arch}.rpm"
rm -f "$out/$name"
"$engine" run --rm --network none \
	--env "VERSION=$version" --env "RPM_ARCH=$arch" --env "SOURCE_DATE_EPOCH=$SOURCE_DATE_EPOCH" \
	--volume "$stage:/stage:ro" \
	--volume "$here/rpm/podup.spec.in:/work/podup.spec.in:ro" \
	--volume "$here/rpm/inner.sh:/work/inner.sh:ro" \
	--volume "$out:/out" \
	"$tag" bash /work/inner.sh
[ -f "$out/$name" ] || { echo "build-rpm: $name was not produced" >&2; exit 1; }
echo "$out/$name"
