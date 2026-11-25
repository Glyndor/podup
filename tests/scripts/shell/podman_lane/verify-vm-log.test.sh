#!/usr/bin/env bash
#
# .github/scripts/podman_lane/verify-vm-log.sh on console logs of the VM that are made here, one
# situation at a time. What it must tell apart:
#
#   - a VM that finished and passed, in the suite leg and in the coverage leg;
#   - a VM that was CUT by its budget before it finished: SLOW RUNNER, with the phase it reached.
#     It is not "no summary" and it is not "no coverage figure": those say the code or the tooling
#     is wrong, and this says the machine was slow (the nightly of 2026-11-23 said the second when
#     it was the first);
#   - a VM that finished and did not report: that one is a defect, and it is said as one;
#   - a VM that finished with a coverage below the floor.
#
# Each plant breaks one rule of the script in a copy and must turn one of the cases red.
# shellcheck disable=SC2016 # the plants hold literal script text
set -u
export LC_ALL=C

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
REAL="$HERE/.github/scripts/podman_lane/verify-vm-log.sh"
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

# The workspace the judge reads its lists from: the real ones, and a list of two known failures.
mkdir -p "$WORK/ws/.github"
cp -R "$HERE/.github/podman_lane" "$WORK/ws/.github/podman_lane"
cp -R "$HERE/.github/scripts" "$WORK/ws/.github/scripts"
printf 'niche::known_one\nniche::known_two\n' >"$WORK/ws/.github/podman_lane/known-failures-6"

# The lines a VM prints, with the CRLF its serial console gives them.
crlf() { sed 's/$/\r/'; }
setup() {
	printf 'DISK_BEGIN value=/dev/vda3 40G 698M 39G 2%% / DISK_END\n'
	printf 'PODMAN_BEGIN version=podman version 6.2.0  RUST=rustc 1.100.0 PODMAN_END\n'
	printf 'LOGDRIVER_BEGIN value=k8s-file LOGDRIVER_END\n'
	printf 'PODUP_PHASE_BEGIN name=setup-done uptime=63 PODUP_PHASE_END\n'
}
suite_done() {
	printf 'test result: ok. 301 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2751.82s\n'
	printf 'PODUP_LEAK_SCAN_BEGIN rc=0 PODUP_LEAK_SCAN_END\n'
	printf 'PODUP_QUADLET_GENERATOR_BEGIN rc=0 PODUP_QUADLET_GENERATOR_END\n'
	printf 'PODUP_TESTS_RC_BEGIN rc=0 PODUP_TESTS_RC_END\n'
	printf 'PODUP_SUMMARY_BEGIN pass=301 fail=0 flaky=0 PODUP_SUMMARY_END\n'
	printf 'PODUP_PHASE_BEGIN name=suite-done uptime=2960 PODUP_PHASE_END\n'
	printf 'PODUP_FAILED_TESTS_BEGIN tests= PODUP_FAILED_TESTS_END\n'
}
cut_line() { printf 'qemu-system-x86_64: terminating on signal 15 from pid 3994 (timeout)\n'; }
done_line() { printf 'PROBE_DONE\n'; }

# run <script> <mode> <qemu rc> <event>, the log in $WORK/log.txt; sets rc and out.
run() {
	mkdir -p "$WORK/t"
	crlf <"$WORK/log.txt" >"$WORK/t/vm.log"
	: >"$WORK/t/summary.md"
	rc=0
	out="$(RUNNER_TEMP="$WORK/t" GITHUB_WORKSPACE="$WORK/ws" GITHUB_STEP_SUMMARY="$WORK/t/summary.md" EVENT="${4:-schedule}" \
		QEMU_RC="$3" VM_SECONDS=4500 VM_IMAGE=test bash "$1" "$2" 2>&1)" || rc=$?
}

cases() { # <script>
	local s="$1"
	{ setup; suite_done; done_line; } >"$WORK/log.txt"
	run "$s" suite 0
	check "a suite that finished and passed: green" "0" "$rc"
	case "$out" in *"GREEN: podup core PASSED on Podman 6.2.0"*) check "...and says so" yes yes ;; *) check "...and says so" green "$out" ;; esac
	check "the phases reach the summary of the run" 1 "$(grep -c 'suite-done: 2960 s' "$WORK/t/summary.md")"

	{ setup; printf 'PODUP_PHASE_BEGIN name=setup-done uptime=63 PODUP_PHASE_END\n'; cut_line; } >"$WORK/log.txt"
	run "$s" suite 124
	check "a suite cut at its budget before it reported: red" "1" "$rc"
	case "$out" in *"::error::SLOW RUNNER"*"suite VM was cut at its budget of 4500 seconds"*) check "...and it says SLOW RUNNER, with the budget" yes yes ;; *) check "...and it says SLOW RUNNER, with the budget" slow "$out" ;; esac
	case "$out" in *"Last phase reached: setup-done 63"*) check "...and names the phase it reached" yes yes ;; *) check "...and names the phase it reached" phase "$out" ;; esac
	case "$out" in *"no PODUP_SUMMARY"*) check "...and does not say the suite did not report" no yes ;; *) check "...and does not say the suite did not report" yes yes ;; esac

	{ setup; cut_line; } >"$WORK/log.txt"
	run "$s" suite 0
	check "the signal of timeout in the log is enough, even if the status was lost: red, slow" "1 1" "$rc $(printf '%s' "$out" | grep -c 'SLOW RUNNER')"

	{ setup; suite_done; cut_line; } >"$WORK/log.txt"
	run "$s" suite 124
	check "cut after the work was done and reported, with no end marker: slow, because it did not finish" "1" "$rc"

	{ setup; suite_done; cut_line; done_line; } >"$WORK/log.txt"
	run "$s" suite 124
	check "killed after it finished (the end marker is there): judged on its work, green" "0" "$rc"

	{ setup; done_line; } >"$WORK/log.txt"
	run "$s" suite 0
	check "a VM that ended and reported no summary: red, as a defect" "1" "$rc"
	case "$out" in *"no PODUP_SUMMARY from VM"*) check "...and says the suite did not report" yes yes ;; *) check "...and says the suite did not report" no-summary "$out" ;; esac
	case "$out" in *"SLOW RUNNER"*) check "...and not that the runner was slow" no yes ;; *) check "...and not that the runner was slow" yes yes ;; esac

	{ setup; suite_done; done_line; } | sed 's/pass=301 fail=0/pass=100 fail=0/' >"$WORK/log.txt"
	run "$s" suite 0
	check "a suite that ran a third of itself: below the baseline, red" "1" "$rc"

	{ setup; printf 'PODUP_PHASE_BEGIN name=coverage-tool-installed uptime=135 PODUP_PHASE_END\n'; printf 'PODUP_COVERAGE=90.67%%\n'; done_line; } >"$WORK/log.txt"
	run "$s" coverage 0
	check "a coverage run that finished with 90.67 %: green" "0" "$rc"
	case "$out" in *"coverage of podup on Podman 6.2.0 was measured and clears its floor"*) check "...and says so" yes yes ;; *) check "...and says so" green "$out" ;; esac

	{ setup; printf 'PODUP_COVERAGE=70.1%%\n'; done_line; } >"$WORK/log.txt"
	run "$s" coverage 0
	check "a coverage run that finished below the floor: red" "1" "$rc"
	case "$out" in *"below the 88% nightly floor"*) check "...and names the floor" yes yes ;; *) check "...and names the floor" floor "$out" ;; esac

	{ setup; done_line; } >"$WORK/log.txt"
	run "$s" coverage 0
	check "a coverage run that finished and gave no figure: red, as a missing figure" "1" "$rc"
	case "$out" in *"no Podman 6 coverage figure"*) check "...and says so" yes yes ;; *) check "...and says so" no-figure "$out" ;; esac

	{ setup; printf 'PODUP_PHASE_BEGIN name=coverage-tool-installed uptime=135 PODUP_PHASE_END\n'; cut_line; } >"$WORK/log.txt"
	run "$s" coverage 124
	check "a coverage run cut at its budget: red, and slow" "1" "$rc"
	case "$out" in *"SLOW RUNNER"*"coverage VM"*) check "...and it says SLOW RUNNER and which VM" yes yes ;; *) check "...and it says SLOW RUNNER and which VM" slow "$out" ;; esac
	case "$out" in *"no Podman 6 coverage figure"*) check "...and not that there was no figure" no yes ;; *) check "...and not that there was no figure" yes yes ;; esac
	case "$out" in *"Last phase reached: coverage-tool-installed 135"*) check "...naming the phase it reached" yes yes ;; *) check "...naming the phase it reached" phase "$out" ;; esac

	{ setup; printf 'PODUP_COVERAGE=70.1%%\n'; done_line; } >"$WORK/log.txt"
	run "$s" coverage 0 workflow_dispatch
	check "the floor does not gate a dispatch" "0" "$rc"


	# What the VM says about its failures must be there, and must agree with itself.
	fails() { # <pass> <fail> <rc> <list> [no-marker]
		setup
		[ "$3" = none ] || printf 'PODUP_TESTS_RC_BEGIN rc=%s PODUP_TESTS_RC_END\n' "$3"
		printf 'PODUP_LEAK_SCAN_BEGIN rc=0 PODUP_LEAK_SCAN_END\nPODUP_QUADLET_GENERATOR_BEGIN rc=0 PODUP_QUADLET_GENERATOR_END\n'
		printf 'PODUP_SUMMARY_BEGIN pass=%s fail=%s flaky=0 PODUP_SUMMARY_END\n' "$1" "$2"
		[ "${5:-}" = no-marker ] || printf 'PODUP_FAILED_TESTS_BEGIN tests=%s PODUP_FAILED_TESTS_END\n' "$4"
	}
	{ fails 299 2 101 "niche::known_one,niche::known_two"; done_line; } >"$WORK/log.txt"
	run "$s" suite 0
	check "two failures, both named, both on the list of the known: green" "0" "$rc"
	{ fails 301 0 0 "" no-marker; done_line; } >"$WORK/log.txt"
	run "$s" suite 0
	check "the pair of markers of the failing tests is missing: red, an absent list is not an empty one" "1" "$rc"
	case "$out" in *"no PODUP_FAILED_TESTS_BEGIN/END pair"*) check "...and says so" yes yes ;; *) check "...and says so" pair "$out" ;; esac
	{ fails 301 0 0 ""; } >"$WORK/log.txt"
	run "$s" suite 0
	check "a green suite that never printed its end marker: red, with any status of qemu" "1" "$rc"
	case "$out" in *"did not reach its end marker"*) check "...and says it did not reach the end" yes yes ;; *) check "...and says it did not reach the end" end "$out" ;; esac
	{ fails 299 2 0 ""; done_line; } >"$WORK/log.txt"
	run "$s" suite 0
	check "the suite counted two failures and names none: red" "1" "$rc"
	case "$out" in *"counted 2 failed and the list of failing tests is empty"*) check "...and says so" yes yes ;; *) check "...and says so" empty "$out" ;; esac
	{ fails 301 0 101 ""; done_line; } >"$WORK/log.txt"
	run "$s" suite 0
	check "the suite ended with a status other than 0, counted none and names none: red" "1" "$rc"
	case "$out" in *"ended with status 101 and names no failing test"*) check "...and says so" yes yes ;; *) check "...and says so" status "$out" ;; esac
	{ fails 299 2 101 "niche::known_one"; done_line; } >"$WORK/log.txt"
	run "$s" suite 0
	check "two failures counted and one named, though known: red, the numbers disagree" "1" "$rc"
	case "$out" in *"counted 2 failed and the list names 1"*) check "...and says so" yes yes ;; *) check "...and says so" count "$out" ;; esac
	{ fails 300 1 101 "niche::known_one,niche::known_two"; done_line; } >"$WORK/log.txt"
	run "$s" suite 0
	check "one failure counted and two named: red as well" "1" "$rc"
	{ fails 300 1 none "niche::known_one"; done_line; } >"$WORK/log.txt"
	run "$s" suite 0
	check "a suite that never reported its status: red" "1" "$rc"

	{ printf 'No space left on device\n'; setup; } >"$WORK/log.txt"
	run "$s" suite 0
	check "a full disk is red" "1" "$rc"
	run "$s" nothing 0
	check "a mode that is neither is refused" "2" "$rc"
}

echo "== the script"
cases "$REAL"

echo "== the plants"
plant() { # <description> <old text, once> <new text>
	local copy="$WORK/plant-$RANDOM.sh" before_fail="$fail" before_pass="$pass"
	if ! python3 - "$REAL" "$copy" "$2" "$3" <<'PYEOF'
import sys
src, dst, old, new = sys.argv[1:5]
text = open(src).read()
if text.count(old) != 1:
    sys.exit(3)
open(dst, "w").write(text.replace(old, new))
PYEOF
	then
		check "the plant applies: $1" applied not-applied
		return
	fi
	cases "$copy" >/dev/null 2>&1
	local caught=$((fail - before_fail))
	fail="$before_fail"
	pass="$before_pass"
	check "plant: $1" yes "$([ "$caught" -gt 0 ] && echo yes || echo no)"
}
plant "a cut is never slow" 'if { [ "${QEMU_RC:-}" = 124 ]' 'if false && { [ "${QEMU_RC:-}" = 124 ]'
plant "only the exit status counts, not the signal in the log" "|| grep -qE 'terminating on signal 15 from pid [0-9]+ \\(timeout\\)' \"\$L\"; }" '; }'
plant "a cut after the end marker is slow" "&& ! tr -d '\\r' <\"\$L\" | grep -qxE 'PROBE_DONE'; then" '; then'
plant "a VM that ended is slow" "&& ! tr -d '\\r' <\"\$L\" | grep -qxE 'PROBE_DONE'; then" '&& false; then'
plant "the phase it reached is not named" 'Last phase reached: ${last:-none (it was cut before the setup finished)}.' 'Last phase reached: unknown.'
plant "the floor not applied in the coverage leg" 'bash "$GITHUB_WORKSPACE/.github/scripts/podman_lane/coverage-floor.sh" "$VER" "$EVENT" "$COV" || exit 1' 'true'
plant "an absent list of failing tests is read as an empty one" 'if [ -z "$FAILED_LISTS" ]; then' 'if false; then'
plant "the end marker is not required" "if ! tr -d '\\r' <\"\$L\" | grep -qxE 'PROBE_DONE'; then" 'if false; then'
plant "failures that are not named pass" 'if [ "$FAIL" -gt 0 ] && [ "$IDS" -eq 0 ]; then' 'if false; then'
plant "a status that is not 0, with no names, passes" 'if [ "$TESTS_RC" != 0 ] && [ "$IDS" -eq 0 ]; then' 'if false; then'
plant "a count that differs from the names passes" 'if [ "$IDS" -ne "$FAIL" ]; then' 'if false; then'
plant "a missing status of the suite passes" '[ -n "$TESTS_RC" ] ||' 'true ||'
plant "the baseline not applied" '[ "$PASS" -ge "$BASELINE" ] ||' 'true ||'

echo
echo "passed: $pass  failed: $fail"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
