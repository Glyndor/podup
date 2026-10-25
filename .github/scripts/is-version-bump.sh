#!/usr/bin/env bash
#
# Usage: is-version-bump.sh BASE HEAD
#
# Exit 0 only when BASE..HEAD is exclusively a podup version bump, so the
# podman lane can skip the VM. Exit 1 for anything else, including every
# error: this answer lets a pull request past an expensive gate, so it
# fails closed. A version bump touches `Cargo.toml`, `Cargo.lock` and
# `debian/changelog` and nothing else, and the script checks the CONTENT
# of each change, not only the names, because a file called Cargo.toml can
# carry a dependency change that the VM must see.
#
# Every condition is mandatory:
#   a. at least one file changed, and every one is Cargo.toml, Cargo.lock
#      or debian/changelog.
#   b. Cargo.toml: the first `version = ...` line must sit inside
#      [package] and match `version = "X.Y.Z"` exactly in both revisions,
#      and the two files must be identical once that line is normalised.
#   c. Cargo.lock: the same, for the `version` line right after
#      `name = "podup"`, which must occur exactly once.
#   d. debian/changelog: HEAD is BASE with whole lines put in front of it
#      (nothing earlier is edited or removed) and starts with `podup (`.
#   e. a changed changelog needs a changed Cargo.toml version with it.
set -uo pipefail

no() {
	echo "not a pure version bump: $*" >&2
	exit 1
}

[ "$#" -eq 2 ] || no "usage: is-version-bump.sh BASE HEAD"
base="$1"
head="$2"
semver='^version = "[0-9]+\.[0-9]+\.[0-9]+"$'

for rev in "$base" "$head"; do
	git rev-parse --verify --quiet "${rev}^{commit}" >/dev/null \
		|| no "cannot resolve $rev"
done

# a. The names.
changed="$(git diff --name-only --no-renames "$base" "$head")" \
	|| no "cannot diff $base..$head"
[ -n "$changed" ] || no "empty range"
while IFS= read -r f; do
	case "$f" in
		Cargo.toml | Cargo.lock | debian/changelog) ;;
		*) no "$f is not part of a version bump" ;;
	esac
done <<<"$changed"

work="$(mktemp -d)" || no "cannot create a temporary directory"
trap 'rm -rf "$work"' EXIT

# Print the original version line, then the file with that line replaced by
# a fixed one. Fails when the line is missing, ambiguous or in the wrong
# place. MODE is toml (first `version` line, must be under [package]) or
# lock (the line after `name = "podup"`, exactly one such package).
normalise() { # <mode> <file>
	awk -v mode="$1" '
		{ cur = $0 }
		/^\[/ { sect = $0 }
		pending {
			pending = 0
			if (cur ~ /^version[ \t]*=/) { ver = cur; found++; cur = "version = \"0.0.0\"" }
			else bad = 1
		}
		mode == "toml" && !found && cur ~ /^version[ \t]*=/ {
			if (sect != "[package]") bad = 1
			ver = cur; found++; cur = "version = \"0.0.0\""
		}
		mode == "lock" && $0 == "name = \"podup\"" { pending = 1 }
		{ out[n++] = cur }
		END {
			if (pending || bad || found != 1) exit 1
			print ver
			for (i = 0; i < n; i++) print out[i]
		}
	' "$2"
}

# Fetch FILE at both revisions and check the two are the same apart from
# the version line, which must be plain X.Y.Z on both sides.
same_but_version() { # <mode> <path>
	git show "$base:$2" >"$work/base" 2>/dev/null || no "$2 is missing in $base"
	git show "$head:$2" >"$work/head" 2>/dev/null || no "$2 is missing in $head"
	normalise "$1" "$work/base" >"$work/base.n" || no "$2 in $base has no usable version line"
	normalise "$1" "$work/head" >"$work/head.n" || no "$2 in $head has no usable version line"
	head -n 1 "$work/base.n" | grep -Eq "$semver" || no "$2 version in $base is not plain X.Y.Z"
	head -n 1 "$work/head.n" | grep -Eq "$semver" || no "$2 version in $head is not plain X.Y.Z"
	cmp -s <(tail -n +2 "$work/base.n") <(tail -n +2 "$work/head.n") \
		|| no "$2 changes more than the version"
}

has() { printf '%s\n' "$changed" | grep -Fxq "$1"; }

# b.
if has Cargo.toml; then same_but_version toml Cargo.toml; fi
# c.
if has Cargo.lock; then same_but_version lock Cargo.lock; fi

# d. and e.
if has debian/changelog; then
	has Cargo.toml || no "debian/changelog changed without a version change in Cargo.toml"
	git show "$base:debian/changelog" >"$work/cl.base" 2>/dev/null || no "debian/changelog is missing in $base"
	git show "$head:debian/changelog" >"$work/cl.head" 2>/dev/null || no "debian/changelog is missing in $head"
	bs="$(wc -c <"$work/cl.base")"
	hs="$(wc -c <"$work/cl.head")"
	[ "$bs" -gt 0 ] || no "debian/changelog is empty in $base"
	added=$((hs - bs))
	[ "$added" -gt 0 ] || no "debian/changelog did not grow"
	cmp -s "$work/cl.base" <(tail -c "$bs" "$work/cl.head") \
		|| no "debian/changelog edits or removes an earlier line"
	[ "$(head -c "$added" "$work/cl.head" | tail -c 1 | od -An -c | tr -d ' ')" = '\n' ] \
		|| no "debian/changelog adds a partial line"
	first="$(head -n 1 "$work/cl.head")"
	case "$first" in
		"podup ("*) ;;
		*) no "debian/changelog does not start with a podup entry" ;;
	esac
fi

echo "a pure version bump: $(printf '%s\n' "$changed" | tr '\n' ' ')"
exit 0
