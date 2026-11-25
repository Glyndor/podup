#!/usr/bin/env bash
#
# Runs INSIDE the pinned Arch Linux container. Writes .PKGINFO and .MTREE for
# the staged tree and packs everything into one tar compressed with zstd, in a
# fixed order, with every owner and time set, so the same input is the same
# bytes. The format is the one makepkg writes: pacman reads the package, not
# the tool that made it.
#
# Reads: /stage (read only: root/ and files.txt)
# Env:   VERSION, SOURCE_DATE_EPOCH
set -euo pipefail

: "${VERSION:?}" "${SOURCE_DATE_EPOCH:?}"
pkg=podup
file="${pkg}-${VERSION}-1-x86_64.pkg.tar.zst"

work=/tmp/pkg
rm -rf "$work"
mkdir -p "$work"
cp -a /stage/root/. "$work/"
cd "$work"

# Installed size, as makepkg counts it: the bytes of the regular files.
size=0
while read -r _mode _path _sha bytes; do size=$((size + bytes)); done </stage/files.txt

cat >.PKGINFO <<INFO
# Generated for the Glyndor release of ${pkg} ${VERSION}
pkgname = ${pkg}
pkgbase = ${pkg}
xdata = pkgtype=pkg
pkgver = ${VERSION}-1
pkgdesc = Run Compose projects on rootless Podman through its native libpod API
url = https://glyndor.net/projects/podup
builddate = ${SOURCE_DATE_EPOCH}
packager = Glyndor <noreply@glyndor.net>
size = ${size}
arch = x86_64
license = MIT
depend = podman>=6.2.0
INFO
touch -d "@${SOURCE_DATE_EPOCH}" .PKGINFO

# Members, in order: every directory and file, sorted. Directories are listed
# on their own (no recursion), as makepkg does.
find . -mindepth 1 ! -name .PKGINFO ! -name .MTREE -print0 | LC_ALL=C sort -z | sed -z 's|^\./||' >/tmp/members

bsdtar --create --file - --format=mtree \
	--options '!all,use-set,type,uid,gid,mode,time,size,sha256,link' \
	--no-recursion --null --files-from /tmp/members .PKGINFO \
	| gzip -n -9 >.MTREE
touch -d "@${SOURCE_DATE_EPOCH}" .MTREE

{ printf '.PKGINFO\0.MTREE\0'; cat /tmp/members; } >/tmp/order

bsdtar --create --file - --format=gnutar --no-recursion --null --files-from /tmp/order \
	--uid 0 --gid 0 --uname root --gname root --numeric-owner \
	| zstd --quiet -19 -T1 >"/out/$file"
chmod 0644 "/out/$file"
