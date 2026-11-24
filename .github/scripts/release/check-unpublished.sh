#!/usr/bin/env bash
#
# Usage: check-unpublished.sh TAG MODE
# MODE is `release` for a real release and `rehearsal` for a `dry_run`.
#
# A release is immutable: a real run refuses a tag that is already published, before
# anything is built, since `gh release create` at the end would refuse it only after
# every build. A rehearsal publishes nothing, so it may name a tag that is already
# published, and that is the safest tag to rehearse on: nothing about it can still
# change. The tag still has to exist and be annotated (check-tag.sh); this only decides
# what a published release means.
#
# The list of published tags is read with `gh api` (GH_TOKEN and GH_REPO set); a test
# puts a file in PODUP_PUBLISHED_TAGS instead. Exit 0 to go on, 1 to stop. A MODE that is
# neither word stops: a typo must not turn the refusal off.
set -euo pipefail
export LC_ALL=C

[ "$#" -eq 2 ] || {
	echo "usage: check-unpublished.sh TAG MODE" >&2
	exit 2
}
tag="$1"
mode="$2"
case "$mode" in release | rehearsal) ;; *)
	echo "::error::the mode is release or rehearsal, not '$mode'" >&2
	exit 2
	;;
esac

if [ -n "${PODUP_PUBLISHED_TAGS:-}" ]; then
	published="$(cat "$PODUP_PUBLISHED_TAGS")"
else
	published="$(gh api --paginate "repos/${GH_REPO:?GH_REPO}/releases" --jq '.[] | select(.draft == false) | .tag_name')"
fi

if grep -Fxq -- "$tag" <<<"$published"; then
	if [ "$mode" = rehearsal ]; then
		echo "$tag is already published; a rehearsal publishes nothing, so it goes on."
		exit 0
	fi
	echo "::error::Release $tag is already published; refusing to rebuild or publish it again." >&2
	exit 1
fi
echo "$tag is not published yet."
