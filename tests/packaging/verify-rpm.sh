#!/usr/bin/env bash
#
# What the rpm channel reads and what a user sees, for one built package.
#
#   verify-rpm.sh RPM STAGE_DIR VERSION RPM_ARCH
#
# 1. The header fields of the channel's contract, printed with the command the
#    contract names, byte for byte, plus Provides, Requires, Obsoletes and
#    Conflicts, the file name, and the size cap.
# 2. For x86_64 only (the binary cannot run elsewhere without an emulator):
#    a clean install in AlmaLinux 9, the oldest distribution the channel
#    serves, `podup --version`, `rpm -V`, every installed file against the
#    files the .deb carries, removal, and nothing left behind.
#
# Containers run with no network. Needs podman (or CONTAINER_ENGINE).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
rpm="${1:?rpm}"
stage="${2:?stage dir}"
version="${3:?version}"
arch="${4:?arch}"
engine="${CONTAINER_ENGINE:-podman}"
# shellcheck disable=SC1091 # a sibling file, not followed by the lint
. "$here/../../.github/scripts/packaging/images.env"

fail() { echo "verify-rpm: $*" >&2; exit 1; }

name="podup-${version}-1.glyndor.any.${arch}.rpm"
[ "$(basename "$rpm")" = "$name" ] || fail "the file is named $(basename "$rpm"), expected $name"
if [ ! -f "$rpm" ] || [ -L "$rpm" ]; then fail "$rpm is not a regular file"; fi
[ "$(stat -c %s "$rpm")" -le $((300 * 1024 * 1024)) ] || fail "$name is over 300 MiB"
rpm_dir="$(cd "$(dirname "$rpm")" && pwd)"

query() { # image args...
	local image="$1"
	shift
	"$engine" run --rm --network none --volume "$rpm_dir:/pkg:ro" "$image" rpm "$@" "/pkg/$name"
}

# --- 1. the header, as the channel prints it ---------------------------------
want="podup|${version}|1.glyndor.any|(none)|${arch}|Glyndor|Glyndor <noreply@glyndor.net>|4"
got="$(query "$RPM_QUERY_IMAGE" -qp --qf '%{NAME}|%{VERSION}|%{RELEASE}|%{EPOCH}|%{ARCH}|%{VENDOR}|%{PACKAGER}|%{RPMFORMAT}\n')"
[ "$got" = "$want" ] || fail "header fields differ:
  want $want
  got  $got"
echo "ok    header fields"

# The oldest rpm the channel serves reads the same package.
got_old="$(query "$RPM_BUILD_BASE" -qp --qf '%{NAME}|%{VERSION}|%{RELEASE}|%{EPOCH}|%{ARCH}|%{VENDOR}|%{PACKAGER}\n')"
[ "$got_old" = "${want%|4}" ] || fail "rpm 4.16 reads other fields: $got_old"
echo "ok    rpm 4.16 reads it"

want_arch_provide="podup(x86-64)"
[ "$arch" = aarch64 ] && want_arch_provide="podup(aarch-64)"
provides="$(query "$RPM_QUERY_IMAGE" -qp --provides | sed 's/ = .*//' | LC_ALL=C sort | paste -sd' ')"
[ "$provides" = "podup $want_arch_provide" ] || fail "Provides: $provides"
echo "ok    provides"

none="$(query "$RPM_QUERY_IMAGE" -qp --obsoletes --conflicts)"
[ "$none" = "" ] || [ "$none" = "(none)" ] || fail "Obsoletes or Conflicts: $none"
echo "ok    no obsoletes or conflicts"

requires="$(query "$RPM_QUERY_IMAGE" -qp --requires | grep -v '^rpmlib(' | paste -sd'|')"
[ "$requires" = "podman >= 6.2.0" ] || fail "Requires besides rpmlib: $requires"
echo "ok    requires podman >= 6.2.0 and nothing else"

# --- the files, against the .deb's -----------------------------------------
all_entries="$(query "$RPM_QUERY_IMAGE" -qp --qf '[%{FILEMODES:octal} %{FILENAMES} %{FILEUSERNAME} %{FILEGROUPNAME}\n]')"
listing="$(grep '^100' <<<"$all_entries" | LC_ALL=C sort -k2)"
owned_dirs="$(grep -v '^100' <<<"$all_entries" | LC_ALL=C sort -k2)"
[ "$owned_dirs" = "40755 /usr/share/doc/podup root root
40755 /usr/share/licenses/podup root root" ] || fail "directories the package owns: $owned_dirs"
expected="$(awk '{ printf "100%s /%s root root\n", substr($1, 2), $2 }' "$stage/files.txt" | LC_ALL=C sort -k2)"
[ "$listing" = "$expected" ] || fail "the files in the package are not the staged ones:
$(diff <(echo "$expected") <(echo "$listing") || true)"
echo "ok    file list, modes and owners"

if [ "$arch" != x86_64 ]; then
	echo "skip  install and run: the $arch binary needs an emulator"
	exit 0
fi

# --- 2. clean install, run, verify, remove ---------------------------------
# The script below is for the container's shell, not this one.
# shellcheck disable=SC2016
"$engine" run --rm --network none \
	--volume "$rpm_dir:/pkg:ro" --volume "$stage:/stage:ro" \
	--env "VERSION=$version" --env "NAME=$name" \
	"$RPM_BUILD_BASE" bash -Eeuo pipefail -c '
	trap "echo \"failed in the container script at line \$LINENO: \$BASH_COMMAND\" >&2" ERR
	# AlmaLinux 9 has podman 5.x, below the 6.2.0 the package asks for, and the
	# Glyndor repository that carries 6.2.0 is the channel itself. The install
	# here ignores that one dependency; the field was checked above.
	snapshot() { find / -xdev \( -path /proc -o -path /sys -o -path /tmp -o -path /var/lib/rpm -o -path /var/cache -o -path /run \) -prune -o -print | LC_ALL=C sort; }
	snapshot >/tmp/before
	rpm --install --nodeps "/pkg/$NAME"
	rpm -q podup | grep -qx "podup-$VERSION-1.glyndor.any.x86_64"
	podup --version | grep -q "$VERSION"
	# --nodeps: the podman dependency is the one the install ignored above.
	test -z "$(rpm -V --nodeps podup)"
	while read -r mode path sha size; do
		test "$(sha256sum "/$path" | cut -d" " -f1)" = "$sha"
		test "$(stat -c %a "/$path")" = "${mode#0}"
		test "$(stat -c %s "/$path")" = "$size"
	done </stage/files.txt
	test -f /usr/share/man/man1/podup.1.gz
	rpm --erase podup
	! rpm -q podup >/dev/null 2>&1
	while read -r _mode path _sha _size; do test ! -e "/$path"; done </stage/files.txt
	test ! -e /usr/share/licenses/podup
	test ! -e /usr/share/doc/podup
	test ! -e /usr/share/zsh/site-functions/_podup
	snapshot >/tmp/after
	left="$(comm -3 /tmp/before /tmp/after)"
	if [ -n "$left" ]; then echo "left behind or changed: $left" >&2; exit 1; fi
'
echo "ok    clean install in AlmaLinux 9, podup --version, rpm -V, files equal the .deb's, removal leaves nothing"
