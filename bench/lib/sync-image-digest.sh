#!/usr/bin/env bash
#
# Usage: bench/lib/sync-image-digest.sh
#
# Writes the alpine digest of bench/scenarios/build/Dockerfile into every
# scenario that pins the same image. The Dockerfile is the one place Dependabot
# watches, so it is the source: when its pull request bumps the digest, run this
# and push the result to that branch. The scenarios keep the digest written out
# in full because run.sh pre-pulls whatever `docker.io/...@sha256:...` it finds
# there, and every tool must read the same compose input.
#
# `tests/repository/supply_chain/bench_pins_one_image_digest.rs` fails while any
# scenario differs from the Dockerfile.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
source_file="$root/scenarios/build/Dockerfile"

digests="$(grep -ohE 'alpine@sha256:[0-9a-f]{64}' "$source_file" | LC_ALL=C sort -u)"
if [ "$(printf '%s\n' "$digests" | grep -c .)" -ne 1 ]; then
	echo "expected exactly one alpine digest in $source_file, found:" >&2
	printf '%s\n' "$digests" >&2
	exit 1
fi

find "$root/scenarios" -type f \( -name '*.yaml' -o -name '*.yml' \) -print0 \
	| xargs -0 sed -i -E "s#alpine@sha256:[0-9a-f]{64}#${digests}#g"
echo "every scenario pins ${digests}"
