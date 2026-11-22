#!/usr/bin/env bash
# The installer's Podman precheck compares a local engine against the same
# floor the binary applies at run time: major, minor and patch, no upper bound.
# It used to compare the major alone, so a release between the old major and the
# floor read as supported. The real check_podman is taken out of install.sh and
# run against a stand-in `podman` that prints the version each case names.
set -uo pipefail
export LC_ALL=C

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
INSTALL_SH="$HERE/install.sh"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
pass=0
fail=0

check() {
	if [[ "$2" == "$3" ]]; then
		echo "ok    $1"
		pass=$((pass + 1))
	else
		echo "FAIL  $1 (expected '$2', got '$3')"
		fail=$((fail + 1))
	fi
}

# The production functions, not copies of them.
HELPERS="$WORK/helpers.sh"
# The three loggers are one-line functions; the rest are blocks.
grep -E '^log_(info|ok|error)\(\)' "$INSTALL_SH" >> "$HELPERS"
for name in fail compare_versions check_podman; do
	awk "/^${name}\\(\\) *\\{/,/^}\$/" "$INSTALL_SH" >> "$HELPERS"
done
for name in log_info log_ok log_error fail compare_versions check_podman; do
	check "$name is found in install.sh" 1 "$(grep -c "^${name}()" "$HELPERS")"
done
# shellcheck disable=SC1090
source "$HELPERS"

# The floor the installer declares, read from the script rather than restated.
PODMAN_MIN_VERSION="$(sed -n 's/^PODMAN_MIN_VERSION=//p' "$INSTALL_SH")"
check "install.sh declares a three-part floor" 1 \
	"$([[ "$PODMAN_MIN_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] && echo 1 || echo 0)"

# A `podman` on PATH whose `--version` prints $1, or exits non-zero for "broken".
make_podman() {
	local dir="$WORK/bin-$1"
	rm -rf "$dir" && mkdir -p "$dir"
	if [[ "$2" == "broken" ]]; then
		printf '#!/bin/sh\nexit 3\n' > "$dir/podman"
	else
		printf '#!/bin/sh\nprintf "%%s\\n" "%s"\n' "$2" > "$dir/podman"
	fi
	chmod +x "$dir/podman"
	echo "$dir"
}

# precheck NAME OUTPUT [skip [floor]]: run check_podman with PATH holding only
# the stand-in, print "rc|output". `floor` replaces the installer's own floor
# for the one call.
precheck() {
	local dir out rc=0
	dir="$(make_podman "$1" "$2")"
	out="$(PATH="$dir" SKIP_PODMAN_CHECK="${3:-}" PODMAN_MIN_VERSION="${4:-$PODMAN_MIN_VERSION}" check_podman 2>&1)" || rc=$?
	printf '%s|%s' "$rc" "$out"
}

said() { [[ "$1" == *"$2"* ]] && echo 1 || echo 0; }

refused() {
	local result rc
	result="$(precheck "$1" "podman version $1" "${2:-}")"
	rc="${result%%|*}"
	check "$1 is refused" 1 "$rc"
	check "$1 is refused for the floor, naming it and the version found" 1 \
		"$(said "$result" "Local Podman $1 is below the >= $PODMAN_MIN_VERSION floor")"
}

accepted() {
	local result rc
	result="$(precheck "$1" "podman version $1")"
	rc="${result%%|*}"
	check "$1 is accepted" 0 "$rc"
	check "$1 is reported as meeting the floor" 1 \
		"$(said "$result" "meets the >= $PODMAN_MIN_VERSION requirement")"
}

# Below the floor: the whole of Podman 4 and 5, and the 6.x releases before it.
refused 4.9.3
refused 5.0.0
refused 5.4.2
refused 5.7.0
refused 5.8.5
refused 5.99.99
refused 6.0.0
refused 6.1.4
refused 6.1.99

# At the floor and above it. 6.10.0 is above 6.2.0 only as a number.
accepted "$PODMAN_MIN_VERSION"
accepted 6.2.1
accepted 6.2.10
accepted 6.3.0
accepted 6.10.0
accepted 7.0.0
accepted 10.0.0

# The installer's floor ends in .0, so no real version can fall short on the
# patch alone. A floor with a patch shows the third part is compared too.
result="$(precheck patch-below "podman version 6.2.3" "" 6.2.4)"
check "6.2.3 is refused by a 6.2.4 floor" 1 "${result%%|*}"
check "6.2.3 is refused for that floor" 1 "$(said "$result" "below the >= 6.2.4 floor")"
result="$(precheck patch-at "podman version 6.2.4" "" 6.2.4)"
check "6.2.4 meets a 6.2.4 floor" 0 "${result%%|*}"
result="$(precheck patch-above "podman version 6.2.10" "" 6.2.4)"
check "6.2.10 meets a 6.2.4 floor" 0 "${result%%|*}"
result="$(precheck minor-above "podman version 6.3.0" "" 6.2.4)"
check "6.3.0 meets a 6.2.4 floor whatever its patch" 0 "${result%%|*}"

# A pre-release or build suffix is judged by the release it leads up to.
result="$(precheck dev-above "podman version 6.3.0-dev")"
check "6.3.0-dev is accepted" 0 "${result%%|*}"
result="$(precheck rc-at-floor "podman version ${PODMAN_MIN_VERSION}-rc1")"
check "${PODMAN_MIN_VERSION}-rc1 is accepted" 0 "${result%%|*}"
result="$(precheck dev-below "podman version 6.1.4-dev")"
check "6.1.4-dev is refused" 1 "${result%%|*}"
check "6.1.4-dev is refused for the floor" 1 "$(said "$result" "below the >= $PODMAN_MIN_VERSION floor")"

# What the check cannot read, or cannot find, it skips with a note; it never
# refuses on a guess.
result="$(precheck unreadable "podman version nonsense")"
check "an unreadable version is skipped" 0 "${result%%|*}"
check "an unreadable version says so" 1 "$(said "$result" "Could not parse the local Podman version")"
result="$(precheck two-parts "podman version 6.2")"
check "a version without a patch is skipped, not accepted as 6.2.0" 1 "$(said "$result" "Could not parse the local Podman version")"
result="$(precheck broken broken)"
check "a podman that fails --version is skipped" 0 "${result%%|*}"
check "a podman that fails --version says so" 1 "$(said "$result" "could not be read")"
EMPTY="$WORK/empty"
mkdir -p "$EMPTY"
result="$(PATH="$EMPTY" check_podman 2>&1)"
check "no local podman is not a failure" 0 "$?"
check "no local podman names the floor" 1 "$(said "$result" "podup needs Podman >= $PODMAN_MIN_VERSION to run")"

# --skip-podman-check lets an engine below the floor through, loudly.
result="$(precheck skipped "podman version 5.7.0" 1)"
check "--skip-podman-check lets 5.7.0 through" 0 "${result%%|*}"
check "--skip-podman-check says it skipped" 1 "$(said "$result" "skipped by request")"

echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
