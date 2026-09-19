#!/usr/bin/env bash
#
# `check-branch-conclusion.sh` decides whether the newest completed run of a
# workflow on a branch is a pass. It is the only piece of behaviour in the
# branch-health guard, and the rest of it is a YAML wiring the shell script
# is fed by `gh api`. A test that exercises the wiring but not the decision
# proves nothing about the decision.
#
# The harness here plants API answers on stdin and asserts exit code plus
# the matching summary line. Three states the brief calls out by name --
# `success` passes, `failure` fails, an empty list reports "no run on
# record" -- are covered with the same JSON the GitHub API returns today,
# plus `cancelled` and `skipped`, which the reusable's comment says it
# treats as failures (the comment above the script's `case` is the
# contract this test pins).
#
# Each plant is a complete `repos/.../runs` response, not just the array,
# because the script reads `.workflow_runs` and would behave differently
# against a bare array vs. an envelope. Building plants out of the
# envelope is the cheapest way to read what the script reads. Every run
# carries `created_at` so the script can sort by it; the cases below
# plant pages whose first item is not the newest on purpose to prove the
# sort runs inside the script rather than relying on the API order.
#
# H1 through H6 cover the behaviour measured on 2026-09-19: a cancelled
# push run underneath a newer one must not flip the guard red, the page
# must be sorted inside the script because the API is not always newest-
# first, and a page whose every entry is cancelled fails with a message
# that says so. H6 pins `skipped` as a failure: a skipped run is not
# passed over the way a cancelled one is.
#
# Requires: bash, jq, the script under test, nothing else.
set -u

cd "$(dirname "$0")/../.." || exit 1
script=.github/scripts/check-branch-conclusion.sh
[ -x "$script" ] || { echo "FAIL  $script is not executable in git"; exit 1; }
command -v jq >/dev/null 2>&1 || { echo "FAIL  jq is required to run this test"; exit 1; }

pass=0; fail=0

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

# Run the script under test against a planted API answer. The script
# reads WORKFLOW and BRANCH from the environment, so the harness passes
# them in.
run_with() { # <workflow> <branch> <json-on-stdin>
	WORKFLOW="$1" BRANCH="$2" bash "$script" <<<"$3"
}

# Each plant is a complete response from the workflow-runs endpoint,
# carrying `created_at`, `conclusion`, and `html_url` for every run.
# Times are ISO 8601 strings, the shape the GitHub API returns, so the
# script's `sort_by(.created_at)` has the same input it would in
# production. The `created_at` ordering also doubles as the test name:
# "success 11:00" means a success run whose `created_at` is 11:00.

plant_success='{"workflow_runs":[{"id":1,"conclusion":"success","html_url":"https://x/r/1","created_at":"2026-09-19T11:00:00Z"}]}'
plant_failure='{"workflow_runs":[{"id":2,"conclusion":"failure","html_url":"https://x/r/2","created_at":"2026-09-19T11:00:00Z"}]}'
plant_cancelled='{"workflow_runs":[{"id":3,"conclusion":"cancelled","html_url":"https://x/r/3","created_at":"2026-09-19T11:00:00Z"}]}'
plant_skipped='{"workflow_runs":[{"id":4,"conclusion":"skipped","html_url":"https://x/r/4","created_at":"2026-09-19T11:00:00Z"}]}'
plant_empty='{"workflow_runs":[],"total_count":0}'

# --- success passes ---
out=$(run_with "ci.yml" "main" "$plant_success"); rc=$?
check "success on main exits 0" "0" "$rc"
check "success on main prints the url" "https://x/r/1" \
	"$(printf '%s\n' "$out" | grep -oE 'https://x/r/[0-9]+' || true)"

# --- failure fails ---
out=$(run_with "ci.yml" "main" "$plant_failure"); rc=$?
check "failure on main exits 1" "1" "$rc"
check "failure on main names the conclusion" "failure" \
	"$(printf '%s\n' "$out" | grep -oE 'conclusion=f[a-z]+' | head -1 | cut -d= -f2)"

# --- a page whose only run is cancelled fails with the all-cancelled
# message. The lone-cancelled case used to read "was cancelled"; a single
# cancelled row on the page is now the all-cancelled case. ---
out=$(run_with "ci.yml" "main" "$plant_cancelled"); rc=$?
check "lone cancelled on main exits 1" "1" "$rc"
case "$out" in
	*"All 1 completed runs"*"were cancelled"*) check "lone cancelled says all 1 were cancelled" "yes" "yes" ;;
	*) check "lone cancelled says all 1 were cancelled" "all-1-were-cancelled" "$(printf '%s\n' "$out" | head -1)" ;;
esac

# --- skipped fails and explains why ---
out=$(run_with "ci.yml" "main" "$plant_skipped"); rc=$?
check "skipped on main exits 1" "1" "$rc"
case "$out" in
	*"was skipped"*) check "skipped on main is named" "yes" "yes" ;;
	*) check "skipped on main is named" "skipped-named" "$(printf '%s\n' "$out" | head -1)" ;;
esac

# --- empty history fails with "no completed run on record" ---
out=$(run_with "ci.yml" "main" "$plant_empty"); rc=$?
check "empty history on main exits 1" "1" "$rc"
case "$out" in
	*"No completed run"*) check "empty history says no run on record" "yes" "yes" ;;
	*) check "empty history says no run on record" "no-run-on-record" "$(printf '%s\n' "$out" | head -1)" ;;
esac

# --- the script's branch parameter actually surfaces in the message ---
# Same input, different branch argument: the failure message names the
# branch the caller asked about, not whichever the harness happens to use.
out_main=$(run_with "ci.yml" "main" "$plant_failure")
out_develop=$(run_with "ci.yml" "develop" "$plant_failure")
case "$out_main" in
	*"on main"*) check "failure message names branch=main" "yes" "yes" ;;
	*) check "failure message names branch=main" "branch=main" "$(printf '%s\n' "$out_main" | head -1)" ;;
esac
case "$out_develop" in
	*"on develop"*) check "failure message names branch=develop" "yes" "yes" ;;
	*) check "failure message names branch=develop" "branch=develop" "$(printf '%s\n' "$out_develop" | head -1)" ;;
esac

# --- the script refuses to run without WORKFLOW or BRANCH ---
out=$(WORKFLOW='' BRANCH=main bash "$script" <<<"$plant_success" 2>/dev/null); rc=$?
check "missing WORKFLOW is fatal" "1" "$rc"
out=$(WORKFLOW=ci.yml BRANCH='' bash "$script" <<<"$plant_success" 2>/dev/null); rc=$?
check "missing BRANCH is fatal" "1" "$rc"

# --- the negative control: a planted response the parser is asked to
# reject, with a conclusion that does not exist in the API. The script
# falls through to the default branch, which is "not success" rather
# than silently passing. ---
plant_unknown='{"workflow_runs":[{"id":5,"conclusion":"made_up","html_url":"https://x/r/5","created_at":"2026-09-19T11:00:00Z"}]}'
out=$(run_with "ci.yml" "main" "$plant_unknown"); rc=$?
check "unknown conclusion exits 1" "1" "$rc"
case "$out" in
	*"unexpected conclusion made_up"*) check "unknown conclusion is named" "yes" "yes" ;;
	*) check "unknown conclusion is named" "unexpected-made_up" "$(printf '%s\n' "$out" | head -1)" ;;
esac

# ===========================================================================
# H1: a page with a cancelled run and an older success. Exit 0, the verdict
# line carries the success URL and not the cancelled one, and the script
# prints one line saying it passed over 1 cancelled run. The cancelled URL
# must appear in the "passed over" line (so the operator can find it) but
# NOT on the verdict line.
# ===========================================================================
plant_h1='{"workflow_runs":[
  {"id":10,"conclusion":"cancelled","html_url":"https://x/r/10","created_at":"2026-09-19T12:00:00Z"},
  {"id":11,"conclusion":"success","html_url":"https://x/r/11","created_at":"2026-09-19T11:00:00Z"}
]}'
out=$(run_with "ci.yml" "main" "$plant_h1"); rc=$?
check "H1: cancelled over success exits 0" "0" "$rc"
verdict_line="$(printf '%s\n' "$out" | grep -E 'Newest completed run of')"
case "$verdict_line" in
	*"https://x/r/11"*) check "H1: verdict line carries the success URL" "yes" "yes" ;;
	*) check "H1: verdict line carries the success URL" "https://x/r/11" "$verdict_line" ;;
esac
case "$verdict_line" in
	*"https://x/r/10"*) check "H1: verdict line does NOT carry the cancelled URL" "no-10" "$verdict_line" ;;
	*) check "H1: verdict line does NOT carry the cancelled URL" "yes" "yes" ;;
esac
case "$out" in
	*"Passed over 1 cancelled run"*) check "H1: script says 1 cancelled run was passed over" "yes" "yes" ;;
	*) check "H1: script says 1 cancelled run was passed over" "passed-over-1" "$(printf '%s\n' "$out" | head -1)" ;;
esac

# ===========================================================================
# H2: a page with a cancelled run and an older failure. Exit 1, names the
# failure URL on the verdict line. The cancelled URL must NOT be on the
# verdict line, the same way it is not in H1.
# ===========================================================================
plant_h2='{"workflow_runs":[
  {"id":20,"conclusion":"cancelled","html_url":"https://x/r/20","created_at":"2026-09-19T12:00:00Z"},
  {"id":21,"conclusion":"failure","html_url":"https://x/r/21","created_at":"2026-09-19T11:00:00Z"}
]}'
out=$(run_with "ci.yml" "main" "$plant_h2"); rc=$?
check "H2: cancelled over failure exits 1" "1" "$rc"
verdict_line="$(printf '%s\n' "$out" | grep -E 'Newest completed run of')"
case "$verdict_line" in
	*"https://x/r/21"*) check "H2: verdict line names the failure URL" "yes" "yes" ;;
	*) check "H2: verdict line names the failure URL" "https://x/r/21" "$verdict_line" ;;
esac
case "$verdict_line" in
	*"https://x/r/20"*) check "H2: verdict line does NOT carry the cancelled URL" "no-20" "$verdict_line" ;;
	*) check "H2: verdict line does NOT carry the cancelled URL" "yes" "yes" ;;
esac

# ===========================================================================
# H3: a page whose every run is cancelled. Exit 1 with a message that
# names the count and says there is no verdict, so an operator can tell
# the failure is "we could not find a verdict" rather than "the verdict
# is failure".
# ===========================================================================
plant_h3='{"workflow_runs":[
  {"id":30,"conclusion":"cancelled","html_url":"https://x/r/30","created_at":"2026-09-19T13:00:00Z"},
  {"id":31,"conclusion":"cancelled","html_url":"https://x/r/31","created_at":"2026-09-19T12:00:00Z"},
  {"id":32,"conclusion":"cancelled","html_url":"https://x/r/32","created_at":"2026-09-19T11:00:00Z"}
]}'
out=$(run_with "ci.yml" "main" "$plant_h3"); rc=$?
check "H3: all-cancelled page exits 1" "1" "$rc"
case "$out" in
	*"All 3 completed runs"*"were cancelled"*) check "H3: says all 3 were cancelled" "yes" "yes" ;;
	*) check "H3: says all 3 were cancelled" "all-3-were-cancelled" "$(printf '%s\n' "$out" | head -1)" ;;
esac

# ===========================================================================
# H4: an unsorted page where the API happens to return the OLDER success
# first and the NEWER failure second. The script must sort by
# `created_at` itself and read the failure as the newest run; without the
# sort it would pass on the success it sees at index [0]. This is the
# case the schedule-freshness gate tripped on in 2026-09-19.
# ===========================================================================
plant_h4='{"workflow_runs":[
  {"id":40,"conclusion":"success","html_url":"https://x/r/40","created_at":"2026-09-19T10:00:00Z"},
  {"id":41,"conclusion":"failure","html_url":"https://x/r/41","created_at":"2026-09-19T12:00:00Z"}
]}'
out=$(run_with "ci.yml" "main" "$plant_h4"); rc=$?
check "H4: unsorted success-then-failure exits 1" "1" "$rc"
verdict_line="$(printf '%s\n' "$out" | grep -E 'Newest completed run of')"
case "$verdict_line" in
	*"https://x/r/41"*) check "H4: verdict line names the failure URL despite unsorted page" "yes" "yes" ;;
	*) check "H4: verdict line names the failure URL despite unsorted page" "https://x/r/41" "$verdict_line" ;;
esac

# ===========================================================================
# H5: the inverse of H4. Older failure first, newer success second.
# Without the sort the script would fail on the failure it sees at
# index [0]; with the sort it picks the success and passes.
# ===========================================================================
plant_h5='{"workflow_runs":[
  {"id":50,"conclusion":"failure","html_url":"https://x/r/50","created_at":"2026-09-19T10:00:00Z"},
  {"id":51,"conclusion":"success","html_url":"https://x/r/51","created_at":"2026-09-19T12:00:00Z"}
]}'
out=$(run_with "ci.yml" "main" "$plant_h5"); rc=$?
check "H5: unsorted failure-then-success exits 0" "0" "$rc"
verdict_line="$(printf '%s\n' "$out" | grep -E 'Newest completed run of')"
case "$verdict_line" in
	*"https://x/r/51"*) check "H5: verdict line names the success URL despite unsorted page" "yes" "yes" ;;
	*) check "H5: verdict line names the success URL despite unsorted page" "https://x/r/51" "$verdict_line" ;;
esac

# ===========================================================================
# H6: a page whose newest run is skipped, with an older success under it.
# Skipped is NOT passed over (that is `cancelled`'s treatment), so the
# script must fail on the skipped run and not on the success.
# ===========================================================================
plant_h6='{"workflow_runs":[
  {"id":60,"conclusion":"skipped","html_url":"https://x/r/60","created_at":"2026-09-19T12:00:00Z"},
  {"id":61,"conclusion":"success","html_url":"https://x/r/61","created_at":"2026-09-19T11:00:00Z"}
]}'
out=$(run_with "ci.yml" "main" "$plant_h6"); rc=$?
check "H6: skipped over success exits 1" "1" "$rc"
case "$out" in
	*"was skipped"*"https://x/r/60"*) check "H6: names the skipped URL, not the success" "yes" "yes" ;;
	*) check "H6: names the skipped URL, not the success" "skipped-url" "$(printf '%s\n' "$out" | head -1)" ;;
esac

echo
echo "passed: $pass  failed: $fail"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
