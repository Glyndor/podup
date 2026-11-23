#!/usr/bin/env bash
#
# Refuse a pull request whose commits carry a personal email address.
#
# Every commit in <base>..<head> is checked for four things: the author address, the
# committer address, every Signed-off-by, and every Co-authored-by. Each has to be a GitHub noreply
# address (or GitHub's own noreply@github.com and support@github.com, the latter
# used by older Dependabot commits). A trailer is checked for every address it
# carries, and one with none is refused. Addresses are never printed: they
# are the thing being protected, and a CI log is public too.
#
# Why it exists: in November 2026 thirty-four commits went to a public repository
# signed off with a personal address. The dco check only asks for "a name and an
# address", so it passed every one of them, and history cannot be taken back.
#
# Merge commits are read like any other: the ones GitHub writes carry its own identity,
# which is allowed, and one written on a machine with a personal address is a leak.
#
# Usage: check-commit-identity.sh <base> <head>

set -euo pipefail

base="${1:?base}"; head="${2:?head}"

allowed='^([0-9]+\+)?[A-Za-z0-9][A-Za-z0-9-]*(\[bot\])?@users\.noreply\.github\.com$|^(noreply|support)@github\.com$'

bad=0
flag() { # <sha> <what>
	echo "::error::commit $1: $2 is not a GitHub noreply address"
	bad=1
}

# Trailers as git reads them, and as a reader of the message does. Git accepts a key
# followed by a space before the colon (`Signed-off-by :`) and a value folded onto
# continuation lines, and a sign-off that is not in the last paragraph is still a line a
# human reads as one. So every line of the message that starts with the key, whatever
# follows the key before the colon, is read with its continuation lines joined.
trailer_values() { # <sha> <key>
	git show -s --format='%B' "$1" | awk -v key="$2" '
		function flush() { gsub(/^[ \t]+|[ \t]+$/, "", cur); gsub(/[ \t]+/, " ", cur); if (cur != "") print cur; cur = "" }
		BEGIN { want = tolower(key) }
		{
			low = tolower($0)
			if (substr(low, 1, length(want)) == want && substr(low, length(want) + 1) ~ /^[ \t]*:/) {
				flush(); cur = substr($0, length(want) + 1); sub(/^[ \t]*:/, "", cur); next
			}
			if (cur != "" && $0 ~ /^[ \t]+[^ \t]/) { cur = cur " " $0; next }
			flush()
		}
		END { flush() }'
}

# Every commit, merges and the root of a range included: the author and the committer
# address of each, and the trailers of each. The committer is a second place a personal
# address lives (a commit rebased or amended on a machine with another identity), and
# GitHub's own merges and squashes carry its noreply address, which is allowed.
while IFS= read -r sha; do
	[ -n "$sha" ] || continue
	author="$(git show -s --format='%ae' "$sha")"
	printf '%s\n' "$author" | grep -qE "$allowed" || flag "$sha" "the author address"
	committer="$(git show -s --format='%ce' "$sha")"
	printf '%s\n' "$committer" | grep -qE "$allowed" || flag "$sha" "the committer address"

	for key in Signed-off-by Co-authored-by; do
		# Two readings, both checked: git's own (unfolded) and the lines of the message.
		values="$(
			git show -s --format="%(trailers:key=$key,valueonly,unfold)" "$sha"
			trailer_values "$sha" "$key"
		)"
		values="$(printf '%s\n' "$values" | LC_ALL=C sort -u)"
		while IFS= read -r value; do
			[ -n "$value" ] || continue
			# Every address-shaped token in the value, not just the one inside the
			# last <...>: a trailer can carry two, and a personal one in front of a
			# noreply one must not pass. A value with no address is refused too.
			addrs="$(printf '%s\n' "$value" | grep -oE '[^<>[:space:]]+@[^<>[:space:]]+' || true)"
			if [ -z "$addrs" ]; then flag "$sha" "a $key with no address"; continue; fi
			while IFS= read -r addr; do
				printf '%s\n' "$addr" | grep -qE "$allowed" || flag "$sha" "a $key address"
			done <<< "$addrs"
		done <<< "$values"
	done
done < <(git rev-list "$base..$head")

[ "$bad" -eq 0 ] || { echo "Use your GitHub noreply address (git config user.email)." >&2; exit 1; }
echo "every commit uses a GitHub noreply address"
