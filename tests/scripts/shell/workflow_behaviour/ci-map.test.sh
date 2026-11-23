#!/usr/bin/env bash
#
# The map of what a change triggers, held as an exact list in both directions.
#
# ci_map/map.py asks the five judges (the Rust lanes, the root build, the Debian
# package, the workflow lint and the Podman VM) and the path filters of the other
# workflows about every tracked file that is not a document, and compares each
# answer with ci_map/expected.txt and ci_map/triggers.txt. A file no pattern covers
# fails, and so does a file whose answer moved: into "nothing to run" when it can
# change the code, or into a lane that has nothing to look at. Each case below
# plants one such move in a copy of the judges or of the workflows and expects the
# comparison to go red; the real tree is expected to pass.
#
# Requires: python3 with PyYAML, git.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
map="$HERE/tests/scripts/shell/workflow_behaviour/ci_map/map.py"
plants="$HERE/tests/scripts/shell/workflow_behaviour/ci_map/plants.py"
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

check "the real tree matches the map" 0 "$(python3 "$map" --check >"$WORK/real.log" 2>&1; echo $?)"
count="$(python3 "$plants" --count)"
i=0
while [ "$i" -lt "$count" ]; do
	rm -rf "$WORK/checks" "$WORK/workflows"
	cp -r "$HERE/.github/scripts/checks" "$WORK/checks"
	cp -r "$HERE/.github/workflows" "$WORK/workflows"
	python3 "$plants" "$i" "$WORK/checks" "$WORK/workflows" || check "plant $i finds its text" planted "not found"
	check "caught: $(python3 "$plants" --describe "$i")" 1 \
		"$(MAP_ONLY="$(python3 "$plants" --affects "$i")" MAP_CHECKS="$WORK/checks" MAP_WORKFLOWS="$WORK/workflows" python3 "$map" --check >"$WORK/out.log" 2>&1; echo $?)"
	i=$((i + 1))
done

echo
echo "passed: $pass  failed: $fail"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
