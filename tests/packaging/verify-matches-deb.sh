#!/usr/bin/env bash
#
# The staged payload the rpm and the pacman package are made from carries what
# the .deb carries: the same bytes at the same paths, with one path moved (the
# zsh completion, from Debian's `vendor-completions` to `site-functions`), the
# Debian-format copyright file replaced by the repository LICENSE, and nothing
# else added.
#
#   verify-matches-deb.sh DEB STAGE_DIR LICENSE
#
# Reads the .deb with dpkg-deb, not with the code under test.
set -euo pipefail

deb="${1:?deb}"
stage="${2:?stage dir}"
licence="$(realpath "${3:?LICENSE}")"

fail() { echo "verify-matches-deb: $*" >&2; exit 1; }

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
dpkg-deb -x "$deb" "$tmp/deb"

# From the .deb: path sha256, regular files only, copyright left out, zsh moved.
(
	cd "$tmp/deb"
	find . -type f ! -path ./usr/share/doc/podup/copyright -printf '%P\n' | LC_ALL=C sort | while read -r path; do
		dest="$path"
		[ "$path" = usr/share/zsh/vendor-completions/_podup ] && dest=usr/share/zsh/site-functions/_podup
		printf '%s %s\n' "$dest" "$(sha256sum "$path" | cut -d' ' -f1)"
	done
	printf 'usr/share/licenses/podup/LICENSE %s\n' "$(sha256sum "$licence" | cut -d' ' -f1)"
) | LC_ALL=C sort >"$tmp/from-deb"

awk '{ print $2, $3 }' "$stage/files.txt" | LC_ALL=C sort >"$tmp/staged"
if ! cmp -s "$tmp/from-deb" "$tmp/staged"; then
	diff "$tmp/from-deb" "$tmp/staged" >&2 || true
	fail "the staged files are not the .deb's"
fi
# And the tree on disk is what the listing says.
(cd "$stage/root" && find . -type f -printf '%P\n' | LC_ALL=C sort) >"$tmp/on-disk"
awk '{ print $1 }' "$tmp/staged" >"$tmp/listed"
cmp -s "$tmp/on-disk" "$tmp/listed" || fail "the staged tree holds other files than files.txt lists"
echo "ok    the staged payload is the .deb's, $(wc -l <"$tmp/staged") files"
