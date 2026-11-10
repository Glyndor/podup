#!/usr/bin/env bash
#
# Usage: check-tag.sh TAG
#
# Run in a checkout of the tag, before anything is built. Fails unless TAG is an
# annotated tag object of this repository that points at the commit checked out,
# so a branch of the same name, a missing tag or a tag on another commit stops
# the release here and not after every build.
set -euo pipefail

tag="${1:?tag}"

if ! git fetch --no-tags origin "+refs/tags/${tag}:refs/tags/${tag}"; then
	echo "::error::${tag} is not a tag of this repository." >&2
	exit 1
fi
if [ "$(git cat-file -t "refs/tags/${tag}")" != "tag" ]; then
	echo "::error::${tag} is not an annotated tag object." >&2
	exit 1
fi
if [ "$(git rev-parse "refs/tags/${tag}^{commit}")" != "$(git rev-parse HEAD)" ]; then
	echo "::error::${tag} does not point at the commit checked out." >&2
	exit 1
fi
echo "${tag} is an annotated tag at the commit checked out."
