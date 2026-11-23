#!/usr/bin/env bash
#
# The exact set of names a release is made of (.github/scripts/release/expected-assets.py) and the
# mode of check-artifact-dir.sh that holds a directory to it. A release cannot be taken back, so the
# files that leave are named one by one from the version the tag was verified to carry: a file that
# is missing, one that is extra, a name of another version and a thing that is not a plain file are
# each a difference, and a directory with nothing in it is not a pass.
#
# Requires: python3.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
EXPECTED="$HERE/.github/scripts/release/expected-assets.py"
CHECK="$HERE/.github/scripts/release/check-artifact-dir.sh"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
pass=0
fail=0
check() { # <description> <expected> <actual>
	if [ "$2" = "$3" ]; then
		echo "ok    $1"
		pass=$((pass + 1))
	else
		echo "FAIL  $1"
		echo "        expected: $2"
		echo "        actual:   $3"
		fail=$((fail + 1))
	fi
}
names() { python3 -I "$EXPECTED" "$1" "$2"; }
status() { python3 -I "$EXPECTED" "$@" >/dev/null 2>&1; echo $?; }

V=6.0.0
# How many names each set has.
check "six binaries" 6 "$(names $V binaries | wc -l)"
check "five packages: two .deb, two .rpm and one pacman package" 5 "$(names $V packages | wc -l)"
check "eight SBOMs: one per binary and per .deb" 8 "$(names $V sboms | wc -l)"
check "what SHA256SUMS lists: binaries, packages, SBOMs, the notices and two installers" 22 "$(names $V checksummed | wc -l)"
check "what the sealing job starts from: those, with the signature of each binary and package" 33 "$(names $V staged | wc -l)"
check "what the sealing job signs: SHA256SUMS, the SBOMs, the notices and the installers" 12 "$(names $V sealed | wc -l)"
check "what is published: every file with its signature" 46 "$(names $V published | wc -l)"
check "every published name but the signatures is a checksummed one or SHA256SUMS" 23 "$(names $V published | grep -vc '\.sig$')"

# The names are the ones the channels and the installers read.
check "a .deb carries the version and the architecture" 1 "$(names $V packages | grep -c '^podup_6.0.0_amd64.deb$')"
check "an rpm is named the way the channel reads it" 1 "$(names $V packages | grep -c '^podup-6.0.0-1.glyndor.any.aarch64.rpm$')"
check "the pacman package has no aarch64 sibling" 0 "$(names $V packages | grep -c 'aarch64.pkg')"
check "the SBOM of a .deb is named after it" 1 "$(names $V sboms | grep -c '^podup_6.0.0_arm64.deb.cdx.json$')"
check "SHA256SUMS is signed and is not in what it lists" 0 "$(names $V checksummed | grep -c '^SHA256SUMS$')"
check "...and is in what is published, with its signature" 2 "$(names $V published | grep -c '^SHA256SUMS')"
check "another version gives other names for the packages" 0 "$(names 6.0.1 packages | grep -c '6.0.0')"
check "the binaries do not carry a version" 1 "$(names 6.0.1 binaries | sort | md5sum | cut -d' ' -f1 | grep -c "$(names 6.0.0 binaries | sort | md5sum | cut -d' ' -f1)")"
check "the lists are sorted with the C collation" 1 "$([ "$(names $V published)" = "$(names $V published | LC_ALL=C sort)" ] && echo 1 || echo 0)"
check "no name is listed twice" 0 "$(names $V published | uniq -d | wc -l)"

# Anything that is not a version, or a set, is refused.
for v in '' 6 6.0 v6.0.0 6.0.0.1 '6.0.0 ' ' 6.0.0' '6.0.x' '6.0.0;id' '../6.0.0' '6.0.0/x' '6.0.0
x' '-1.0.0' '６.0.0'; do
	check "the version '$(printf '%s' "$v" | tr '\n' '~')' is refused" 2 "$(status "$v" binaries)"
done
check "a set that does not exist is refused" 2 "$(status $V nothing)"
check "no arguments is refused" 2 "$(status)"
check "a missing set is refused" 2 "$(status $V)"

# check-artifact-dir.sh holds a directory to a set.
fill() { # <directory> <set>
	rm -rf "$1"
	mkdir -p "$1"
	names "$V" "$2" | while IFS= read -r n; do : >"$1/$n"; done
}
exact() { bash "$CHECK" "$1" exact "$V" "$2" >"$WORK/out" 2>&1; echo $?; }
fill "$WORK/d" staged
check "a directory with exactly the staged set passes" 0 "$(exact "$WORK/d" staged)"
rm "$WORK/d/podup-linux-arm64.sig"
check "a missing signature is refused" 1 "$(exact "$WORK/d" staged)"
check "...and the difference names it" 1 "$(grep -c 'podup-linux-arm64.sig' "$WORK/out")"
fill "$WORK/d" staged; : >"$WORK/d/extra"
check "an extra file is refused" 1 "$(exact "$WORK/d" staged)"
fill "$WORK/d" staged; rm "$WORK/d/NOTICES.html"
check "a missing notices file is refused" 1 "$(exact "$WORK/d" staged)"
fill "$WORK/d" staged; mv "$WORK/d/podup_6.0.0_amd64.deb" "$WORK/d/podup_6.0.1_amd64.deb"
check "a package of another version is refused" 1 "$(exact "$WORK/d" staged)"
fill "$WORK/d" staged; rm "$WORK/d/podup-6.0.0-1-x86_64.pkg.tar.zst.sig"
check "a package with no signature is refused" 1 "$(exact "$WORK/d" staged)"
fill "$WORK/d" staged; : >"$WORK/d/podup-6.0.0-1.glyndor.any.riscv64.rpm"
check "an rpm of an architecture nobody builds is refused" 1 "$(exact "$WORK/d" staged)"
check "the staged set is not the published one" 1 "$(exact "$WORK/d" published)"
fill "$WORK/d" published
check "a directory with exactly what is published passes" 0 "$(exact "$WORK/d" published)"
rm "$WORK/d/SHA256SUMS.sig"
check "published without the signature of the checksums is refused" 1 "$(exact "$WORK/d" published)"
rm -rf "$WORK/e"; mkdir "$WORK/e"
check "an empty directory is not a pass" 1 "$(exact "$WORK/e" published)"
fill "$WORK/d" staged; rm "$WORK/d/install.sh"; ln -s /etc/passwd "$WORK/d/install.sh"
check "a link in the place of a file is refused" 1 "$(exact "$WORK/d" staged)"
fill "$WORK/d" staged; mkdir "$WORK/d/sub"
check "a directory among the files is refused" 1 "$(exact "$WORK/d" staged)"
fill "$WORK/d" staged; mkdir "$WORK/d/sub"; : >"$WORK/d/sub/x"
check "a file below the top is refused" 1 "$(exact "$WORK/d" staged)"
check "a version that is not three numbers is refused" 1 "$(bash "$CHECK" "$WORK/d" exact '6.0' staged >/dev/null 2>&1; echo $?)"
check "no version is refused" 1 "$(bash "$CHECK" "$WORK/d" exact >/dev/null 2>&1; echo $?)"

echo
echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
