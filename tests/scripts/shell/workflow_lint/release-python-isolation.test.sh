#!/usr/bin/env bash
#
# The Python scripts of the release run with -I, and a module planted beside them is not imported.
#
# Python puts the directory of the script it runs first on its path, and these scripts live in a
# directory of the tag that is being released. Whoever can merge one file there, named like a module
# the script imports, would run in the signer, in the check that exists to catch a wrong key, and in
# the check of the set. -I drops the directory of the script (and the user site and the environment).
# Each case copies the scripts, plants a module that writes a marker when it is imported, and runs
# the script the way the workflow runs it and the way it would run without -I: the marker must be
# there only for the second, which is the control that the plant bites.
#
# Requires: python3.
# shellcheck disable=SC2016
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

# <script relative to the repository> <module the script imports> <arguments...>
# Prints "isolated=<marker or not> plain=<marker or not>".
planted() {
	local script="$1" module="$2"
	shift 2
	local d="$WORK/$RANDOM$RANDOM"
	mkdir -p "$d/scripts"
	cp "$HERE/$script" "$d/scripts/"
	printf 'import os\nopen(os.environ["MARKER"], "w").write("imported")\n' >"$d/scripts/$module.py"
	local name
	name="$(basename "$script")"
	rm -f "$d/marker-isolated" "$d/marker-plain"
	(cd "$d" && MARKER="$d/marker-isolated" python3 -I "$d/scripts/$name" "$@" >/dev/null 2>&1)
	(cd "$d" && MARKER="$d/marker-plain" python3 "$d/scripts/$name" "$@" >/dev/null 2>&1)
	printf 'isolated=%s plain=%s' "$([ -e "$d/marker-isolated" ] && echo planted || echo clean)" "$([ -e "$d/marker-plain" ] && echo planted || echo clean)"
}

echo "-- scripts that need only the standard library"
check "expected-assets.py does not import a planted re" "isolated=clean plain=planted" "$(planted .github/scripts/release/expected-assets.py re 6.0.0 binaries)"
check "check-key-slots.py does not import a planted re" "isolated=clean plain=planted" "$(planted .github/scripts/release/check-key-slots.py re "$HERE")"
check "verify-release-assets.py does not import a planted argparse" "isolated=clean plain=planted" "$(planted .github/scripts/release/verify-release-assets.py argparse --help)"
check "stage-payload.py does not import a planted argparse" "isolated=clean plain=planted" "$(planted .github/scripts/packaging/stage-payload.py argparse --help)"

echo "-- scripts that run in the environment of the signer, with cryptography"
# They fail without the library when it is not there; the marker is written on import, before.
check "sign.py does not import a planted base64" "isolated=clean plain=planted" "$(planted .github/scripts/release/sign.py base64 x)"
check "verify-signing-key.py does not import a planted base64" "isolated=clean plain=planted" "$(planted .github/scripts/release/verify-signing-key.py base64 x)"
check "public-key-of.py does not import a planted base64" "isolated=clean plain=planted" "$(planted .github/scripts/release/public-key-of.py base64)"

echo "-- the workflows start each of them with -I"
# Every start of Python in the release is held by release_wiring/wiring.py, which also has a mutant per
# step with the flag taken away; this is the one case that reads the shipped files directly.
unisolated="$(grep -hE '(python3?|"?\$venv_py"?) +[^-| ]|(python3?|"?\$venv_py"?) +-[^I]' \
	"$HERE/.github/workflows/release.yml" "$HERE/.github/workflows/reusable-release-build.yml" \
	"$HERE/.github/workflows/reusable-release-rehearsal.yml" \
	| grep -vE '^\s*#|command -v python|py=python|python3 -I|\$venv_py" -I|-m venv' || true)"
check "no step of the release starts Python without -I" "" "$unisolated"

echo
echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
