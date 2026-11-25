#!/usr/bin/env bash
#
# Usage: verify-vm-log.sh MODE
# MODE is `suite` (the gating suite, the leak scan and the Quadlet generator tests) or
# `coverage` (the instrumented run and its figure). Reads the console log of the VM at
# $RUNNER_TEMP/vm.log and judges it.
#
# Environment: RUNNER_TEMP, GITHUB_WORKSPACE, GITHUB_STEP_SUMMARY, EVENT (the GitHub event
# name), QEMU_RC (the exit status of qemu, 124 when `timeout` cut it), VM_SECONDS (the budget
# the VM was given), VM_IMAGE (what it booted).
#
# A VM that was cut by its budget before it finished is a SLOW RUNNER, said so, and not a
# missing summary or a missing coverage figure: the first scheduled run that enforced the
# floor of the coverage failed with "no Podman 6 coverage figure was reported" when the
# runner took 40 minutes for a suite that needs 41 and the coverage had no time left. What
# that says is the speed of the machine, and what to read is how far the VM got.
# Not pipefail: it is the shell the step had (`bash -e`), and a few of the pipes below end in a grep
# that finds nothing, which is an answer here and not an error.
set -e
export LC_ALL=C

mode="${1:-}"
case "$mode" in suite | coverage) ;; *) echo "usage: verify-vm-log.sh suite|coverage" >&2; exit 2 ;; esac
L="$RUNNER_TEMP/vm.log"
EVENT="${EVENT:-}"

echo "=== key lines ==="; grep -E 'DISK_BEGIN|PODMAN_BEGIN|test result:|PODUP_TESTS_RC_BEGIN|No space' "$L" | tail -10 || true
grep -q 'No space left' "$L" && { echo "::error::VM disk full; run invalid"; exit 1; }

# The phases the VM reported, by its own clock, one line each, in the order it reached them.
phases="$(tr -d '\r' <"$L" | sed -nE 's/.*PODUP_PHASE_BEGIN name=([a-z-]+) uptime=([0-9]+) PODUP_PHASE_END.*/\1 \2/p')"
if [ -n "$phases" ]; then
  {
    echo "### Phases of the $mode VM (seconds since it booted)"
    echo ""
    printf '%s\n' "$phases" | awk '{ printf "- %s: %s s\n", $1, $2 }'
  } >> "$GITHUB_STEP_SUMMARY"
fi
# Cut by its budget before it finished: that is the speed of the runner, and it is said so.
if { [ "${QEMU_RC:-}" = 124 ] || grep -qE 'terminating on signal 15 from pid [0-9]+ \(timeout\)' "$L"; } \
  && ! tr -d '\r' <"$L" | grep -qxE 'PROBE_DONE'; then
  last="$(printf '%s\n' "$phases" | tail -1)"
  echo "::error::SLOW RUNNER: the $mode VM was cut at its budget of ${VM_SECONDS:-?} seconds before it finished. This says how fast the runner was, not that the code is broken."
  echo "  Last phase reached: ${last:-none (it was cut before the setup finished)}. The phases and their budget are in .github/podman_lane/budget; a run that is slow once is run again, a budget that is too small for the machine every night is measured again and raised there." >&2
  exit 1
fi
# A verdict, green or red, is on a VM that reached its end. One that stopped before it (it crashed, it was
# powered off, a status was lost) has not said its last word, whatever status qemu returned, and what it left
# in the log is not a result.
if ! tr -d '\r' <"$L" | grep -qxE 'PROBE_DONE'; then
  echo "::error::the $mode VM did not reach its end marker (PROBE_DONE), so nothing it printed is a result; it stopped before it finished."
  exit 1
fi
VER=$(perl -0777 -ne 'while (/PODMAN_BEGIN\s+version=podman version ([0-9.]+)(?s:(?:(?!PODMAN_BEGIN).)*?)PODMAN_END/g) { print "$1\n" }' "$L" | tail -1)
[ -n "$VER" ] || { echo "::error::Podman not detected in VM"; exit 1; }
# The VM must be able to serve logs. Gate on that, not on which driver
# provides it: where journald cannot open its library every logs request
# 500s, and a specific driver would be required for the wrong reason.
#
# This is not cosmetic. With logs unavailable, the five tests that only
# `unwrap()` the result passed without fetching a byte, because `logs`
# swallowed the 500 and returned success. Green-and-meaningless is worse
# than red, so the signature is a hard failure.
echo "log driver: $(perl -0777 -ne 'while (/LOGDRIVER_BEGIN\s+value=([a-z0-9-]+)(?s:(?:(?!LOGDRIVER_BEGIN).)*?)LOGDRIVER_END/g) { print "$1\n" }' "$L" | tail -1)"
if grep -q 'unable to open a handle to the library' "$L"; then
  echo "::error::the VM could not serve logs (journald cannot open its library); the logs tests would pass without reading anything"
  exit 1
fi
# Guard the VM: it must run a Podman at or above the floor podup requires
# (6.2.0), whatever rawhide ships today.
if [ "$(printf '%s\n6.2.0\n' "$VER" | LC_ALL=C sort -V | head -1)" != "6.2.0" ]; then
  echo "::error::the VM has Podman $VER, below the 6.2.0 floor podup requires"; exit 1
fi

if [ "$mode" = coverage ]; then
  # Coverage, when this run asked for it. Reported into the job summary.
  # Gated on the scheduled path only, on Podman 6. The CI coverage job
  # without Podman sees 79% because every integration test skips itself;
  # the lane's full suite sees ~91.5%. A 88% floor is below the observed
  # value with room for natural drift and above the level where a real
  # regression would hide (#1326). PR runs do not produce a coverage
  # number, so the gate is skipped there. The judgement lives in
  # .github/scripts/podman_lane/coverage-floor.sh, with behaviour
  # tests: it takes the full version (`6.2.0`) and compares its major.
  COV=$(grep -oE 'PODUP_COVERAGE=[^ ]+' "$L" | tail -1 | cut -d= -f2)
  if [ -n "$COV" ]; then
    echo "coverage on Podman $VER (full suite, integration tests included): $COV"
    echo "- **Podman $VER coverage:** $COV" >> "$GITHUB_STEP_SUMMARY"
  fi
  # Called with an empty figure too: on the scheduled run a missing
  # measurement (the VM timing out during the coverage pass) fails the
  # floor instead of skipping it.
  bash "$GITHUB_WORKSPACE/.github/scripts/podman_lane/coverage-floor.sh" "$VER" "$EVENT" "$COV" || exit 1
  echo "GREEN: the coverage of podup on Podman $VER was measured and clears its floor."
  exit 0
fi

# Read the machine-readable summary the VM computed from the FULL cargo
# log (the console here is tail-truncated, so don't recount from it).
SUM=$(perl -0777 -ne 'while (/PODUP_SUMMARY_BEGIN\s+pass=(\d+)(?s:(?:(?!PODUP_SUMMARY_BEGIN).)*?)fail=(\d+)(?s:(?:(?!PODUP_SUMMARY_BEGIN).)*?)flaky=(\d+)\s+PODUP_SUMMARY_END/g) { print "PODUP_SUMMARY pass=$1 fail=$2 flaky=$3\n" }' "$L" | tail -1)
[ -n "$SUM" ] || { echo "::error::no PODUP_SUMMARY from VM on Podman $VER; the suite did not report"; exit 1; }
PASS=$(echo "$SUM" | grep -oE 'pass=[0-9]+' | cut -d= -f2)
FAIL=$(echo "$SUM" | grep -oE 'fail=[0-9]+' | cut -d= -f2)
FLAKY=$(echo "$SUM" | grep -oE 'flaky=[0-9]+' | cut -d= -f2)
PASS=${PASS:-0}; FAIL=${FAIL:-0}; FLAKY=${FLAKY:-0}
echo "Podman $VER: $PASS passed, $FAIL failed ($FLAKY look like connection-drop flakes)"
echo "Podman $VER ran on image: ${VM_IMAGE:-unknown}"
# The run page, not just the log: whoever reads a red lane needs the
# environment next to the counts, and the rawhide leg's environment
# changes under it (#1222).
{
  echo "### Podman $VER: $PASS passed, $FAIL failed, $FLAKY drop-like"
  echo ""
  echo "image: \`${VM_IMAGE:-unknown}\`"
} >> "$GITHUB_STEP_SUMMARY"
# Strip CR: the VM talks to us over `-serial stdio`, so every line of
# this log arrives CRLF-terminated. The counts survive because they are
# extracted with numeric patterns, but the identity list keeps the CR on
# its final element, which then matches nothing in the classified list
# and reports the last failing test as an unexpected regression. That is
# exactly how this gate false-reddened on its first run.
# The list of failing tests, from a pair of markers that must be there: an absent pair is not an empty list.
# The empty list is printed by the VM too, as `tests=` with nothing after it, and that is the only way to say
# that nothing failed.
FAILED_LISTS=$(perl -0777 -ne 'while (/PODUP_FAILED_TESTS_BEGIN\s+tests=((?:(?!PODUP_FAILED_TESTS_BEGIN).)*?)\s+PODUP_FAILED_TESTS_END/sg) { print "LIST:$1\n" }' "$L" | tr -d '\r')
if [ -z "$FAILED_LISTS" ]; then
  echo "::error::no PODUP_FAILED_TESTS_BEGIN/END pair from VM on Podman $VER; the VM did not say which tests failed, and an absent list is not an empty one"
  exit 1
fi
FAILED_TESTS="$(printf '%s\n' "$FAILED_LISTS" | tail -1 | sed 's/^LIST://')"
TESTS_RC=$(perl -0777 -ne 'while (/PODUP_TESTS_RC_BEGIN\s+rc=(\w+)\s+PODUP_TESTS_RC_END/g) { print "$1\n" }' "$L" | tail -1 | tr -d '\r')
[ -n "$TESTS_RC" ] || { echo "::error::no PODUP_TESTS_RC marker from VM on Podman $VER; the status of the suite was not reported"; exit 1; }
IDS=$(printf '%s' "$FAILED_TESTS" | tr ',' '\n' | awk 'NF { n++ } END { print n + 0 }')
# What the suite counted and what it named must say the same, or one of them is wrong: a failure that has no name
# cannot be looked up in the list of the known ones, and a name with no failure is not one.
if [ "$FAIL" -gt 0 ] && [ "$IDS" -eq 0 ]; then
  echo "::error::Podman $VER: the suite counted $FAIL failed and the list of failing tests is empty; a failure that is not named cannot be classified"
  exit 1
fi
if [ "$TESTS_RC" != 0 ] && [ "$IDS" -eq 0 ]; then
  echo "::error::Podman $VER: the suite ended with status $TESTS_RC and names no failing test; it did not end the way a suite that passed does"
  exit 1
fi
if [ "$IDS" -ne "$FAIL" ]; then
  echo "::error::Podman $VER: the suite counted $FAIL failed and the list names $IDS; the two must agree"
  exit 1
fi
if [ -n "$FAILED_TESTS" ]; then
  echo "Failing tests on Podman $VER: $FAILED_TESTS"
fi
# Identity gate, against the classified list of the major the VM booted.
# A count cannot tell a regression from noise: the #1072 regression
# broke one test and left the pass count at 144, far above the floor,
# so nothing went red and it shipped. WHICH tests fail does tell you.
# The major comes from the Podman the VM actually booted, so the day
# rawhide moves to Podman 7 this step asks for a known-failures-7 file
# instead of passing without a gate.
major="${VER%%.*}"
known="$GITHUB_WORKSPACE/.github/podman_lane/known-failures-$major"
if [ ! -f "$known" ]; then
  echo "::error::the VM booted Podman $VER and there is no .github/podman_lane/known-failures-$major to gate it against; add one (an empty list is fine) once a run proves the failing set"
  exit 1
fi
# `comm` compares bytes while `sort` orders by locale, so pin both to
# C. Defensive rather than observed: the first-run false red was the
# CR above, not collation, but the two must agree or comm reports
# present entries as missing.
LC_ALL=C
export LC_ALL
grep -vE '^[[:space:]]*(#|$)' "$known" | tr -d '[:blank:]' | sort -u > "$RUNNER_TEMP/known.txt"
printf '%s' "$FAILED_TESTS" | tr ',' '\n' | sed '/^$/d' | sort -u > "$RUNNER_TEMP/failed.txt"
unexpected="$(comm -23 "$RUNNER_TEMP/failed.txt" "$RUNNER_TEMP/known.txt")"
if [ -n "$unexpected" ]; then
  echo "::error::Podman $VER: these failing tests are not in $(basename "$known"): a test that used to pass is failing, which is a regression, not nested-virt noise:"
  echo "$unexpected" | awk '{print "  " $0}'
  echo "Environment: Podman $VER on ${VM_IMAGE:-an unrecorded image}. Before concluding, check whether the run you are comparing against booted the same one; the rawhide leg usually does not (#1222)."
  echo "If a run proves the failure is environmental, add it to that file with the evidence; do not add it to make this red go away."
  exit 1
fi
stale="$(comm -13 "$RUNNER_TEMP/failed.txt" "$RUNNER_TEMP/known.txt")"
if [ -n "$stale" ]; then
  echo "::notice::Podman $VER: listed as environmentally-broken but passing in this run; drop them from $(basename "$known") once that holds:"
  echo "$stale" | awk '{print "  " $0}'
fi
echo "Podman $VER: every failure is on the classified list ($(wc -l < "$RUNNER_TEMP/known.txt") entries)."
# Gate: the core must pass the baseline in .github/podman_lane/baseline-tests.
# A real regression breaks previously-passing tests and knocks PASS
# below the floor; the baseline is versioned so it is bumped
# deliberately as the suite grows rather than sitting as a magic number
# here.
#
# 2026-07-27: raised 115 -> 165 against a suite that now runs 172 on both
# legs. At 115 the floor had 57 tests of slack, so a suite that executed
# two thirds of itself and reported them all green would have passed the
# only check that can see a half-executed run. 165 keeps a small margin
# for the identity gate's own territory (a handful of classified
# failures must not redden the floor too) without leaving room for a
# third of the suite to vanish unnoticed.
#
# The floor is a second, cruder signal: it catches "fewer tests ran"
# (a suite that half-executed reports few failures and few passes),
# which the identity gate above cannot see.
#
# Why the failure COUNT is still only a warning: it never separated a
# regression from noise: a count of failures that match a
# connection-drop signature would redden runs whose failures carry no
# such signature (an earlier leg failed the same ~10 streaming/attach/
# run tests every run with none). The identity gate replaces it.
#
# Podman 6 went without a list because its failing set carried a
# variable tail that observation would have turned into false reds,
# and the floor it gated on instead reported the leg green through 31
# failures (141 passed, floor 115). That tail was one cause:
# serialising the leg removed it, and its list is now empty, so any
# failure there is a regression. See the file's own header for the
# evidence and for how thin it is.
BASELINE="$(cat "$GITHUB_WORKSPACE/.github/podman_lane/baseline-tests")"
[ "$PASS" -ge "$BASELINE" ] || { echo "::error::core suite degraded on Podman $VER ($PASS passed, baseline >= $BASELINE): a real regression, or fewer tests ran"; exit 1; }
# Leak scan, run by the VM in a separate cargo process after
# engine_integration exited. The marker is the only signal that the
# scan actually ran; a missing marker means the script never reached
# the scan invocation, which is a gate failure on its own (cf. the
# PODUP_SUMMARY check above).
SCAN_RC=$(perl -0777 -ne 'while (/PODUP_LEAK_SCAN_BEGIN\s+rc=(\w+)\s+PODUP_LEAK_SCAN_END/g) { print "$1\n" }' "$L" | tail -1)
[ -n "$SCAN_RC" ] || { echo "::error::no PODUP_LEAK_SCAN marker from VM on Podman $VER; the leak scan did not report"; exit 1; }
if [ "$SCAN_RC" = "nopid" ]; then
  echo "::error::Podman $VER: engine_integration never wrote target/podup-leak-scan-pid, so the leak scan had no run to look for and scanned nothing"
  exit 1
fi
if [ "$SCAN_RC" != "0" ]; then
  echo "::error::Podman $VER: the leak scan reported rc=$SCAN_RC; resources carrying the per-run prefix remain on the host. See the leak-scan log section in the run page."
  exit 1
fi
# The Quadlet generator tests ran in the VM after the leak scan, with
# PODUP_REQUIRE_QUADLET_GENERATOR=1, so a generator that was missing or
# older than the floor is a failure here, not a skip.
GEN_RC=$(perl -0777 -ne 'while (/PODUP_QUADLET_GENERATOR_BEGIN\s+rc=(\w+)\s+PODUP_QUADLET_GENERATOR_END/g) { print "$1\n" }' "$L" | tail -1)
[ -n "$GEN_RC" ] || { echo "::error::no PODUP_QUADLET_GENERATOR marker from VM on Podman $VER; the Quadlet generator tests did not report"; exit 1; }
if [ "$GEN_RC" != "0" ]; then
  echo "::error::Podman $VER: the Quadlet generator tests reported rc=$GEN_RC. See the quadlet generator section in the run page."
  exit 1
fi
if [ "$FAIL" -gt "$FLAKY" ]; then
  echo "::warning::Podman $VER: $FAIL failures, $FLAKY match a connection-drop signature; the rest are the known nested-virt-unstable subset (see the failures section). Not gated until that flakiness is reduced."
fi
echo "GREEN: podup core PASSED on Podman $VER ($PASS tests >= baseline $BASELINE; $FAIL in the nested-virt-noisy subset)."
