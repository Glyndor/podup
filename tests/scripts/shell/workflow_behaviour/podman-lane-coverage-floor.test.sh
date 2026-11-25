#!/usr/bin/env bash
# Behaviour tests for the Podman lane nightly coverage floor, and for the
# workflow step that calls it with the full version the VM booted.
set -uo pipefail
export LC_ALL=C
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
script="$HERE/.github/scripts/podman_lane/coverage-floor.sh"
workflow="$HERE/.github/scripts/podman_lane/verify-vm-log.sh"
template="$HERE/.github/scripts/podman_lane/user-data.tpl"
lane="$HERE/.github/workflows/podman-lane.yml"
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
run_case "a missing figure fails the nightly" 1 6.2.0 schedule ""
run_case "a missing figure on a manual run is left alone" 0 6.2.0 workflow_dispatch ""
run_case "a figure that arrives with a carriage return passes" 0 6.2.0 schedule $'91.5%\r'
run_case "a figure below the floor with a carriage return is refused" 1 6.2.0 schedule $'70%\r'
run_case "a figure that is only a carriage return counts as missing" 1 6.2.0 schedule $'\r'
rc=0
bash "$script" 6.2.0 >/dev/null 2>&1 || rc=$?
check "too few arguments is refused" 1 "$rc"
out="$(bash "$script" 6.2.0 schedule 70% 2>&1)"
check "the refusal names the floor" 1 "$(printf '%s\n' "$out" | grep -cF 'below the 88% nightly floor')"
out="$(bash "$script" 6.2.0 schedule "" 2>&1)"
check "a missing figure is named as missing" 1 "$(printf '%s\n' "$out" | grep -cF 'no Podman 6 coverage figure')"

# The figure goes through the extraction the workflow does, on a log whose
# lines end in CRLF (the VM's serial console), and then through the script.
extract="$(grep -F "COV=\$(grep -oE 'PODUP_COVERAGE=" "$workflow" | head -1 | sed 's/^[[:space:]]*//')"
check "the workflow extracts the figure from the log" 1 "$(printf '%s' "$extract" | grep -c .)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
printf 'noise\r\nPODUP_COVERAGE=91.5%%\r\nPODUP_SUMMARY_END\r\n' >"$tmp/crlf.log"
printf 'noise\r\nPODUP_SUMMARY_END\r\n' >"$tmp/none.log"
rc=0
L="$tmp/crlf.log" bash -c "$extract"'; bash "$0" 6.2.0 schedule "$COV"' "$script" >/dev/null 2>&1 || rc=$?
check "a CRLF log passes the extraction and the floor" 0 "$rc"
rc=0
L="$tmp/none.log" bash -c "$extract"'; bash "$0" 6.2.0 schedule "$COV"' "$script" >/dev/null 2>&1 || rc=$?
check "a log with no figure fails the floor on the nightly" 1 "$rc"

# The workflow must hand the script the full version, not a pre-cut major, and
# must not carry its own copy of the old comparison.
# shellcheck disable=SC2016 # the pattern is literal workflow text
check "the lane calls the floor script" 1 \
	"$(grep -cF 'coverage-floor.sh" "$VER" "$EVENT" "$COV"' "$workflow")"
# shellcheck disable=SC2016 # the pattern is literal workflow text
check "the lane no longer compares the full version with a bare major" 0 \
	"$(grep -cE '\[ "\$VER" = "[0-9]+" \]' "$workflow")"

# The call must not sit inside the `if [ -n "$COV" ]` block: a missing figure
# has to reach the script, which refuses it on the nightly.
# shellcheck disable=SC2016 # the pattern is literal workflow text
check "the lane calls the floor script even with no figure" 1 \
	"$(awk '
		/COV=\$\(grep -oE/ { match($0, /^ */); base = RLENGTH }
		/coverage-floor.sh" "\$VER"/ { match($0, /^ */); if (RLENGTH == base && base > 0) n++ }
		END { print n + 0 }
	' "$workflow")"

# Single Podman major: nothing in the lane selects by it any more.
# shellcheck disable=SC2016 # the pattern is literal workflow text
check "the lane does not guard a matrix major against the VM" 0 \
	"$(grep -cF 'expected Podman ${{ matrix.podman }}' "$lane")"
check "the thread cap is not substituted per major" 0 "$(grep -c '__THREADS__' "$template")"
check "llvm is installed only for runs that measure coverage" 1 \
	"$(grep -cF 'dnf install -y podman rust cargo gcc git __LLVM__' "$template")"

echo
echo "passed: $pass  failed: $fail"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
