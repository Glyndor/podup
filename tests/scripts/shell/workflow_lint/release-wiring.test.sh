#!/usr/bin/env bash
#
# The wiring of release.yml that only shows once everything is built: what the
# release job waits for, the permissions each job is granted, the folder each
# step runs in, `--verify-tag`, what `attest` covers against what
# verify-provenance.sh asks for, and the names the artifacts carry.
#
# release_wiring/wiring.py holds the assertions; release_wiring/mutants.py
# changes the real workflow one fact at a time and fails if any change goes
# unreported. Both read the workflow as it ships.
#
# Requires: python3 with PyYAML.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
DIR="${WIRING_DIR:-$HERE/tests/scripts/shell/workflow_lint/release_wiring}"

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

check "the shipped release.yml has no wiring problem" "" "$(python3 "$DIR/wiring.py" 2>&1)"

out="$(python3 "$DIR/mutants.py" 2>&1)"
check "no mutant of the wiring survives" "" "$(printf '%s\n' "$out" | grep '^SURVIVED' || true)"
check "the mutants ran (at least thirty)" 1 "$(printf '%s\n' "$out" | awk '/^killed/ {print ($2 >= 30)}')"

# The harness itself: a wiring problem that is really there must make this
# script exit non-zero, not only print FAIL. A copy of the assertions with one
# problem planted is run in a child that must not do the same again.
if [ -z "${WIRING_DIR:-}" ]; then
	mkdir -p "$WORK/planted"
	cp "$DIR"/*.py "$WORK/planted/"
	sed -i 's/^    return out$/    return out + ["planted problem"]/' "$WORK/planted/wiring.py"
	sed -i "s#^REPO = .*#REPO = \"$HERE\"#" "$WORK/planted/wiring.py" "$WORK/planted/mutants.py"
	WIRING_DIR="$WORK/planted" bash "${BASH_SOURCE[0]}" >/dev/null 2>&1
	check "a wiring problem makes this test exit non-zero" 1 "$?"
fi

echo
echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
