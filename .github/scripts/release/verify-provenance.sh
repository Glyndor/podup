#!/usr/bin/env bash
#
# Usage: verify-provenance.sh DIR REPO VERSION [NAME...]
#   env: SOURCE_REF     the ref the release was cut from, refs/tags/v<VERSION>
#        SOURCE_COMMIT  the commit that tag was verified to point at
#
# Run before anything is signed: every binary and package of the release, the set that
# expected-assets.py names for VERSION, must carry a build-provenance attestation made by
# this repository's release workflow, FOR that tag and that commit. Naming the workflow
# alone would let any attestation that workflow ever made for the same bytes stand, and the
# commit and the ref are what tie the bytes to the release being published.
#
# With NAME... only those files are verified, and each has to be a binary or a package of VERSION: the signing
# jobs hold one file each and check it before they sign it. Without names, every one of them is.
set -euo pipefail

dir="${1:?directory}"
repo="${2:?owner/repo}"
version="${3:?version}"
ref="${SOURCE_REF:?SOURCE_REF}"
commit="${SOURCE_COMMIT:?SOURCE_COMMIT}"

[ "$ref" = "refs/tags/v${version}" ] || { echo "::error::SOURCE_REF is '$ref', not refs/tags/v${version}" >&2; exit 1; }
[[ "$commit" =~ ^[0-9a-f]{40}$ ]] || { echo "::error::SOURCE_COMMIT is not a full commit" >&2; exit 1; }

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
files="$(python3 -I "$here/expected-assets.py" "$version" binaries; python3 -I "$here/expected-assets.py" "$version" packages)"
if [ "$#" -gt 3 ]; then
	shift 3
	for name in "$@"; do
		grep -Fxq -- "$name" <<<"$files" || { echo "::error::'$name' is not a binary or a package of version $version" >&2; exit 1; }
	done
	files="$(printf '%s\n' "$@")"
fi

cd "$dir"
while IFS= read -r file; do
	[ -n "$file" ] || continue
	gh attestation verify "$file" --repo "$repo" \
		--signer-workflow "$repo/.github/workflows/release.yml" \
		--source-ref "$ref" --source-digest "$commit"
done <<<"$files"
echo "every binary and package carries an attestation of $ref at $commit."
