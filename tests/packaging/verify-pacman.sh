#!/usr/bin/env bash
#
# What the pacman channel reads and what a user sees, for one built package.
#
#   verify-pacman.sh PKG STAGE_DIR VERSION
#
# 1. The members and the .PKGINFO fields of the channel's contract (no network).
# 2. A clean install in Arch Linux with pacman resolving `podman>=6.2.0` from
#    the repositories (this needs the network: a full upgrade first, since the
#    pinned image is older than the repositories), `podup --version`,
#    `pacman -Qkk`, every installed file against the files the .deb carries,
#    removal, and nothing left behind.
#
# Needs podman (or CONTAINER_ENGINE).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
pkg="${1:?package}"
stage="${2:?stage dir}"
version="${3:?version}"
engine="${CONTAINER_ENGINE:-podman}"
# shellcheck disable=SC1091 # a sibling file, not followed by the lint
. "$here/../../.github/scripts/packaging/images.env"

fail() { echo "verify-pacman: $*" >&2; exit 1; }

name="podup-${version}-1-x86_64.pkg.tar.zst"
[ "$(basename "$pkg")" = "$name" ] || fail "the file is named $(basename "$pkg"), expected $name"
if [ ! -f "$pkg" ] || [ -L "$pkg" ]; then fail "$pkg is not a regular file"; fi
[ "$(stat -c %s "$pkg")" -le $((300 * 1024 * 1024)) ] || fail "$name is over 300 MiB"
pkg_dir="$(cd "$(dirname "$pkg")" && pwd)"
stage="$(cd "$stage" && pwd)"

# --- 1. members and .PKGINFO -------------------------------------------------
# The script below is for the container's shell, not this one.
# shellcheck disable=SC2016
"$engine" run --rm --network none \
	--volume "$pkg_dir:/pkg:ro" --env "VERSION=$version" --env "NAME=$name" \
	--env "WANT_SIZE=$(awk '{ s += $4 } END { print s }' "$stage/files.txt")" \
	"$PACMAN_BUILD_BASE" bash -Eeuo pipefail -c '
	trap "echo \"failed in the container script at line \$LINENO: \$BASH_COMMAND\" >&2" ERR
	cd /tmp
	# .PKGINFO is the first member, once, a regular file of at most 64 KiB.
	test "$(bsdtar -tf "/pkg/$NAME" | head -1)" = .PKGINFO
	test "$(bsdtar -tf "/pkg/$NAME" | grep -c "^\.PKGINFO$")" = 1
	bsdtar -tvf "/pkg/$NAME" | head -1 | grep -q "^-"
	test "$(bsdtar -xOf "/pkg/$NAME" .PKGINFO | wc -c)" -le 65536
	# No member with an absolute name, a .. component, or anything that is not a
	# plain file or directory; no more than 100000 entries.
	! bsdtar -tf "/pkg/$NAME" | grep -E "^/|(^|/)\.\.(/|$)"
	! bsdtar -tvf "/pkg/$NAME" | grep -E "^[^-d]"
	test "$(bsdtar -tf "/pkg/$NAME" | wc -l)" -le 100000
	info="$(bsdtar -xOf "/pkg/$NAME" .PKGINFO)"
	field() { grep -E "^$1 = " <<<"$info" | sed "s/^$1 = //"; }
	test "$(field pkgname)" = podup
	test "$(field pkgver)" = "$VERSION-1"
	test "$(field arch)" = x86_64
	test "$(field packager)" = "Glyndor <noreply@glyndor.net>"
	# pkgbase absent or equal; replaces absent; provides and conflicts absent or the package itself.
	pkgbase="$(field pkgbase)"; test -z "$pkgbase" || test "$pkgbase" = podup
	test -z "$(field replaces)"
	test -z "$(field conflict)"
	test -z "$(field provides)"
	test "$(field depend)" = "podman>=6.2.0"
	test -z "$(field backup)"
	! grep -q "^pkgname = glyndor-keyring" <<<"$info"
	# The installed size the package declares is the bytes of its files.
	test "$(field size)" = "$WANT_SIZE"
'
echo "ok    members and .PKGINFO fields"

# --- 2. install in Arch ------------------------------------------------------
# NoExtract in the image's pacman.conf leaves out man pages and documents, which
# are part of what is checked here.
# shellcheck disable=SC2016
"$engine" run --rm \
	--volume "$pkg_dir:/pkg:ro" --volume "$stage:/stage:ro" \
	--env "VERSION=$version" --env "NAME=$name" \
	"$PACMAN_BUILD_BASE" bash -Eeuo pipefail -c '
	trap "echo \"failed in the container script at line \$LINENO: \$BASH_COMMAND\" >&2" ERR
	sed -i "/^NoExtract/d" /etc/pacman.conf
	pacman -Syu --noconfirm --disable-sandbox >/dev/null 2>&1
	# podman first, so what the snapshot compares is podup alone.
	pacman -S --noconfirm --needed --disable-sandbox "podman>=6.2.0" >/dev/null 2>&1
	snapshot() { find / -xdev \( -path /proc -o -path /sys -o -path /tmp -o -path /var/lib/pacman -o -path /var/cache -o -path /run \) -prune -o -print | LC_ALL=C sort; }
	snapshot >/tmp/before
	pacman -U --noconfirm --disable-sandbox "/pkg/$NAME" >/dev/null 2>&1
	test "$(pacman -Q podup)" = "podup $VERSION-1"
	pacman -Q podman | grep -q "^podman 6\."
	podup --version | grep -q "$VERSION"
	# Every file the package owns is intact: pacman -Qkk compares to .MTREE.
	pacman -Qkk podup | tail -1 | grep -q " 0 altered files"
	while read -r mode path sha size; do
		test "$(sha256sum "/$path" | cut -d" " -f1)" = "$sha"
		test "$(stat -c %a "/$path")" = "${mode#0}"
		test "$(stat -c %s "/$path")" = "$size"
	done </stage/files.txt
	pacman -R --noconfirm --disable-sandbox podup >/dev/null 2>&1
	! pacman -Q podup >/dev/null 2>&1
	snapshot >/tmp/after
	left="$(comm -3 /tmp/before /tmp/after)"
	if [ -n "$left" ]; then echo "left behind or changed: $left" >&2; exit 1; fi
'
echo "ok    clean install in Arch Linux with podman from the repositories, podup --version, pacman -Qkk, files equal the .deb's, removal leaves nothing"
