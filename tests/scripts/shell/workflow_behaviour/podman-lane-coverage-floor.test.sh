#!/usr/bin/env bash
# Behaviour tests for the Podman lane nightly coverage floor, and for the
# workflow step that calls it with the full version the VM booted.
set -uo pipefail
export LC_ALL=C
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
script="$HERE/.github/scripts/checks/podman-lane-coverage-floor.sh"
workflow="$HERE/.github/workflows/podman-lane.yml"
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
run_case() { # <description> <expected rc> <version> <event> <coverage>
	local rc=0
	bash "$script" "$3" "$4" "$5" >/dev/null 2>&1 || rc=$?
	check "$1" "$2" "$rc"
}

# The VM reports the full version, so the floor must engage on 6.2.0.
run_case "full version below the floor is refused on the nightly" 1 6.2.0 schedule 87.4%
run_case "the figure is rounded, as the old inline check did" 0 6.2.0 schedule 87.6%
run_case "full version at the floor passes on the nightly" 0 6.2.0 schedule 88.0%
run_case "full version above the floor passes on the nightly" 0 6.2.0 schedule 91.5%
run_case "a later Podman 6 release is gated too" 1 6.10.1 schedule 50%
run_case "a bare major is gated as well" 1 6 schedule 50%
run_case "a manual run is not gated" 0 6.2.0 workflow_dispatch 10%
run_case "another major is not gated" 0 7.0.0 schedule 10%
run_case "a major that merely starts with 6 is not Podman 6" 0 60.1.0 schedule 10%
run_case "an unreadable figure fails the nightly" 1 6.2.0 schedule n/a
run_case "an unreadable figure on a manual run is left alone" 0 6.2.0 workflow_dispatch n/a
run_case "a missing argument is refused" 1 6.2.0 schedule ""
rc=0
bash "$script" 6.2.0 >/dev/null 2>&1 || rc=$?
check "too few arguments is refused" 1 "$rc"
out="$(bash "$script" 6.2.0 schedule 70% 2>&1)"
check "the refusal names the floor" 1 "$(printf '%s\n' "$out" | grep -cF 'below the 88% nightly floor')"

# The workflow must hand the script the full version, not a pre-cut major, and
# must not carry its own copy of the old comparison.
# shellcheck disable=SC2016 # the pattern is literal workflow text
check "the lane calls the floor script" 1 \
	"$(grep -cF 'podman-lane-coverage-floor.sh" "$VER" "${{ github.event_name }}" "$COV"' "$workflow")"
# shellcheck disable=SC2016 # the pattern is literal workflow text
check "the lane no longer compares the full version with a bare major" 0 \
	"$(grep -cE '\[ "\$VER" = "[0-9]+" \]' "$workflow")"

echo
echo "passed: $pass  failed: $fail"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
