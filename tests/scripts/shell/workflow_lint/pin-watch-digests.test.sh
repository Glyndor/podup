#!/usr/bin/env bash
#
# Every pinned value that pin-watch.yml reads from a file is still there to be read. The watcher takes a
# digest from the file that pins it with a pattern; when the pin moves to another file (the builds of the
# release moved into reusable-release-build.yml) a watcher that still reads the old file reads nothing and
# either fails or, worse, keeps comparing a value that no job uses. Each `read_digest <file> '<pattern>'`
# of the workflow has to find exactly one digest in the file it names, the files it names have to exist,
# and the pair that is compared with the lane has to be the one the release builds with.
#
# Requires: python3.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
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

# <root> -> one line per read_digest of pin-watch.yml: "<status> <file>" with status ok, nofile or nodigest.
reads() {
	python3 - "$1" <<'PY'
import os, re, subprocess, sys
root = sys.argv[1]
text = open(os.path.join(root, ".github/workflows/pin-watch.yml")).read()
calls = re.findall(r"read_digest\s+(\S+)\s*\\?\s*\n?\s*'([^']+)'", text)
if not calls:
    print("nocalls -")
for path, pattern in calls:
    full = os.path.join(root, path)
    if not os.path.isfile(full):
        print("nofile", path)
        continue
    found = re.findall(pattern.replace("[a-f0-9]", "[a-f0-9]"), open(full).read())
    print(("ok" if len(found) >= 1 else "nodigest"), path)
PY
}

check "pin-watch reads at least two digests" 1 "$([ "$(reads "$HERE" | grep -vc '^nocalls')" -ge 2 ] && echo 1 || echo 0)"
check "every digest it reads is found in the file it names" "" "$(reads "$HERE" | grep -v '^ok' || true)"
check "it reads the digest of the release build and the one of the lane" 2 "$(reads "$HERE" | grep -c '^ok')"
check "the release build it reads is the file the release calls for its builds" 1 "$(reads "$HERE" | grep -c 'reusable-release-build.yml')"
check "no digest is read from release.yml, which no longer holds one" 0 "$(reads "$HERE" | grep -c ' .github/workflows/release.yml')"

# The same reads over copies with one thing taken away.
fresh() { rm -rf "$WORK/t"; mkdir -p "$WORK/t/.github"; cp -r "$HERE/.github/workflows" "$WORK/t/.github/workflows"; }
fresh; rm "$WORK/t/.github/workflows/reusable-release-build.yml"
check "a file the watcher names that is gone is found" "nofile .github/workflows/reusable-release-build.yml" "$(reads "$WORK/t" | grep -v '^ok')"
fresh; sed -i 's/container: debian:trixie@sha256:/container: debian:trixie@sha999:/' "$WORK/t/.github/workflows/reusable-release-build.yml"
check "a digest that is no longer in the file is found" "nodigest .github/workflows/reusable-release-build.yml" "$(reads "$WORK/t" | grep -v '^ok')"
fresh; sed -i 's/container: debian:trixie@/image: debian:trixie@/' "$WORK/t/.github/workflows/reusable-release-build.yml"
check "a pin whose key was renamed is found" "nodigest .github/workflows/reusable-release-build.yml" "$(reads "$WORK/t" | grep -v '^ok')"
fresh; sed -i 's#read_digest .github/workflows/reusable-release-build.yml#read_digest .github/workflows/release.yml#' "$WORK/t/.github/workflows/pin-watch.yml"
check "a watcher that still reads release.yml for the digest is found" "nodigest .github/workflows/release.yml" "$(reads "$WORK/t" | grep -v '^ok')"
fresh; : >"$WORK/t/.github/workflows/pin-watch.yml"
check "a watcher that reads nothing is found" "nocalls -" "$(reads "$WORK/t")"

echo
echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
