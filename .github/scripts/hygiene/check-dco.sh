#!/usr/bin/env bash
#
# The Developer Certificate of Origin, as a check: every commit a pull request adds, a merge
# aside, must carry a `Signed-off-by: Name <address>` trailer whose address is the one of the
# commit's author (an empty trailer, one with no address and one signed for somebody else are
# not a sign-off), and no merge commit may carry a tree that is not the clean merge of its two
# parents.
#
# The second half is why a merge is not simply exempt. A merge commit is excused from its own
# trailer, so a hand-edited one would carry a change that exists in neither parent and never
# passed review or sign-off. The tree of the merge must equal what `git merge-tree` makes of
# its parents. An octopus merge, a merge whose parents conflict and a merge whose tree cannot
# be recomputed fail closed: a hand-resolved conflict cannot be told from one that smuggled in
# something else. The flow here is squash only, so a merge commit inside a pull request is
# rare and always avoidable by rebasing.
#
# Usage: check-dco.sh <base> <head>

set -euo pipefail

base="${1:?base}"; head="${2:?head}"

commits="$(git rev-list --no-merges "$base..$head")" || {
	echo "::error::could not list $base..$head; the range could not be checked" >&2
	exit 1
}

bad=0
while IFS= read -r sha; do
	[ -n "$sha" ] || continue
	author="$(git show -s --format='%ae' "$sha")" || exit 1
	values="$(git show -s --format='%(trailers:key=Signed-off-by,valueonly,unfold)' "$sha")" || exit 1
	signed=false
	while IFS= read -r value; do
		# `Name <address>`: a name, then one address in angle brackets and nothing after it.
		if [[ "$value" =~ ^[^\<\>]*[^\<\>[:space:]][^\<\>]*\<([^\<\>[:space:]]+@[^\<\>[:space:]]+)\>[[:space:]]*$ ]]; then
			[ "${BASH_REMATCH[1],,}" != "${author,,}" ] || signed=true
		fi
	done <<< "$values"
	if [ "$signed" = false ]; then
		echo "::error::commit $sha has no Signed-off-by: Name <address> trailer with the address of its author"
		bad=1
	fi
done <<< "$commits"
if [ "$bad" -ne 0 ]; then
	echo "Sign off with: git commit -s (or git rebase --signoff)."
	exit 1
fi

merges="$(git rev-list --merges "$base..$head")" || {
	echo "::error::could not list the merge commits of $base..$head; this check fails closed" >&2
	exit 1
}
err="$(mktemp)"
trap 'rm -f "$err"' EXIT
while IFS= read -r merge; do
	[ -n "$merge" ] || continue
	read -r -a parents <<< "$(git show -s --format='%P' "$merge")"
	if [ "${#parents[@]}" -ne 2 ]; then
		echo "::error::merge commit $merge has ${#parents[@]} parents; only an ordinary two-parent merge can be verified"
		bad=1
		continue
	fi
	if ! clean="$(git merge-tree --write-tree "${parents[0]}" "${parents[1]}" 2>"$err")"; then
		echo "::error::merge commit $merge cannot be verified (conflicted or merge-tree failed); rebase the branch instead of merging"
		bad=1
		continue
	fi
	if [ "$clean" != "$(git rev-parse "$merge^{tree}")" ]; then
		echo "::error::the tree of merge commit $merge is not the clean merge of its parents"
		bad=1
	fi
done <<< "$merges"
[ "$bad" -eq 0 ] || exit 1
echo "every commit is signed off and every merge is clean"
