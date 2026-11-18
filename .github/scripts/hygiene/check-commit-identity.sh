#!/usr/bin/env bash
#
# Refuse a pull request whose commits carry a personal email address.
#
# Every commit in <base>..<head> is checked for three things: the author address,
# every Signed-off-by, and every Co-authored-by. Each has to be a GitHub noreply
# address (or GitHub's own noreply@github.com). Addresses are never printed: they
# are the thing being protected, and a CI log is public too.
#
# Why it exists: in November 2026 thirty-four commits went to a public repository
# signed off with a personal address. The dco check only asks for "a name and an
# address", so it passed every one of them, and history cannot be taken back.
#
# Merge commits are skipped; the ones GitHub writes carry its own identity.
#
# Usage: check-commit-identity.sh <base> <head>

set -euo pipefail

base="${1:?base}"; head="${2:?head}"

allowed='^([0-9]+\+)?[A-Za-z0-9][A-Za-z0-9-]*(\[bot\])?@users\.noreply\.github\.com$|^noreply@github\.com$'

bad=0
flag() { # <sha> <what>
	echo "::error::commit $1: $2 is not a GitHub noreply address"
	bad=1
}

while IFS= read -r sha; do
	[ -n "$sha" ] || continue
	author="$(git show -s --format='%ae' "$sha")"
	printf '%s\n' "$author" | grep -qE "$allowed" || flag "$sha" "the author address"

	for key in Signed-off-by Co-authored-by; do
		while IFS= read -r value; do
			[ -n "$value" ] || continue
			addr="$(printf '%s\n' "$value" | sed -n 's/.*<\([^<>]*\)>.*/\1/p')"
			printf '%s\n' "$addr" | grep -qE "$allowed" || flag "$sha" "a $key address"
		done < <(git show -s --format="%(trailers:key=$key,valueonly)" "$sha")
	done
done < <(git rev-list --no-merges "$base..$head")

[ "$bad" -eq 0 ] || { echo "Use your GitHub noreply address (git config user.email)." >&2; exit 1; }
echo "every commit uses a GitHub noreply address"
