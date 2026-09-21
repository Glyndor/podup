#!/usr/bin/env bash
#
# Behaviour tests for `.github/scripts/check-branch-conclusion.sh`.
#
# The script decides whether the run for a branch's CURRENT head was a
# pass. It used to read the newest completed run on the branch, with
# `cancelled` skipped over on the theory a newer push had superseded
# it. The defect measured on 2026-09-20: that logic could answer for a
# commit nobody had tested (let A pass, push B, cancel B's run by
# hand with nothing replacing it, and the script reported A's
# success). The new logic picks the run whose `head_sha` is the
# branch's current head, polls until that run finishes, and treats a
# `cancelled` run for that head as RED.
#
# The harness plants API answers through a stub `gh` on PATH. Each
# call to `gh` increments an index and returns line N of a planted
# responses file (compact JSON, one response per line), so the script's
# polling is exercised by adding more lines than it needs. The stub
# records its argv to a log so the URL the step built can be checked
# for the right filters, the same way the schedule-freshness test
# does.
#
# Cases:
#   - head's run green passes
#   - head's run red fails, naming it
#   - head's run cancelled fails (this used to be passed over)
#   - still running waits one cycle then passes when completed
#   - never appearing fails after the configured attempts, with one
#     fewer sleeps than attempts
#   - an older green run for a different commit is NOT the verdict
#   - a newer pull_request run for the same head does NOT shadow an
#     older push run for that head (the bug measured 2026-09-20 on the
#     5.9.5 release push for commit 83315af)
#   - the inverse: a newer push failure for the head still wins, so
#     the fix is not "prefer whichever succeeded"
#   - the branch head is read from the API on a schedule event when
#     HEAD_SHA is not set in the environment
#
# Plus a handful of structural assertions (missing env vars, unknown
# conclusion, stream split) that survive the new design unchanged.
#
# Requires: bash, jq, python3 for some helpers, the script under test.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
script="$HERE/.github/scripts/check-branch-conclusion.sh"
[ -x "$script" ] || { echo "FAIL  $script is not executable in git"; exit 1; }
command -v jq >/dev/null 2>&1 || { echo "FAIL  jq is required to run this test"; exit 1; }

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

# Stub `gh` and stub `sleep`. `gh` increments a call counter and returns
# line N of STUB_RESPONSES, applying any `--jq` filter with real jq the
# way `gh api --jq` does. `sleep` records its argument and returns, so
# the polling tests don't actually wait. Both write their argv to a log
# (NUL-separated) so the test can read what the script asked for.
#
# The gh stub honours STUB_EXIT_CODE: a test sets it to 1 (or anything
# non-zero) to make every call fail with that exit code, which is what
# exercises the script's poll-loop survival of a transient API failure.
write_stub() {
	mkdir -p "$WORK/bin"
	cat > "$WORK/bin/gh" <<'STUB'
#!/usr/bin/env bash
LOG="${STUB_LOG:?stub log path required}"
RESP="${STUB_RESPONSES:?stub responses file required}"
idx=$(grep -cz . "$LOG" 2>/dev/null || true)
idx="${idx:-0}"
idx=$((idx + 1))
printf '%s\0' "$*" >> "$LOG"
prev=""; filter=""
for arg in "$@"; do
	if [ "$prev" = "--jq" ]; then
		filter="$arg"
	fi
	prev="$arg"
done
val=$(awk -v n="$idx" 'NR==n {print; exit}' "$RESP")
if [ -n "$filter" ]; then
	printf '%s' "$val" | jq -r "$filter"
else
	printf '%s' "$val"
fi
exit "${STUB_EXIT_CODE:-0}"
STUB
	chmod +x "$WORK/bin/gh"
	cat > "$WORK/bin/sleep" <<'STUB'
#!/usr/bin/env bash
printf '%s\0' "$*" >> "${SLEEP_LOG:?sleep log path required}"
STUB
	chmod +x "$WORK/bin/sleep"
}
write_stub

# Build a single run record as compact JSON. The script now filters by
# `event=push` so the verdict must read the run the branch triggered.
# The default `event` for a fixture is `push`; the cases that need a
# different event pass it explicitly as the 8th argument.
make_run() { # <id> <head_sha> <status> <conclusion> <created_at> <run_number> <html_url> [event]
	local event="${8:-push}"
	jq -nc --argjson id "$1" --arg head "$2" --arg status "$3" --arg conclusion "$4" \
		--arg created "$5" --argjson number "$6" --arg url "$7" --arg event "$event" \
		'{id:$id, head_sha:$head, status:$status, conclusion:$conclusion, created_at:$created, run_number:$number, html_url:$url, event:$event}'
}

# Build a page from one or more compact-JSON run records. Each page
# is followed a by a newline, so the stub gh that hands out "line N"
# sees each response on its own line.
make_page() { # <runs...>
	local first=1
	printf '{"workflow_runs":['
	for run in "$@"; do
		if [ "$first" = 1 ]; then
			first=0
		else
			printf ','
		fi
		printf '%s' "$run"
	done
	printf ']}\n'
}

# Run the script under test against planted responses. Combine
# stdout+stderr in `out`, exit code in `rc`. Each call starts from a
# fresh gh.log and sleep.log so call counts are per-invocation.
# MAX_ATTEMPTS and SLEEP_SECONDS inherited from the caller's
# environment shrink the polling bound so the "never appears" case
# does not actually take 8 minutes; the defaults match production.
run_script() { # <responses_file> [head_sha]
	local resp="$1"
	local head="${2:-HEAD123}"
	rm -f "$WORK/gh.log" "$WORK/sleep.log"
	: >"$WORK/gh.log"
	: >"$WORK/sleep.log"
	PATH="$WORK/bin:$PATH" \
		STUB_LOG="$WORK/gh.log" STUB_RESPONSES="$resp" SLEEP_LOG="$WORK/sleep.log" \
		WORKFLOW=ci.yml BRANCH=main REPO=owner/repo HEAD_SHA="$head" GH_TOKEN=dummy \
		MAX_ATTEMPTS="${MAX_ATTEMPTS:-32}" SLEEP_SECONDS="${SLEEP_SECONDS:-15}" \
		bash "$script" 2>&1
}

# Count recorded `gh` calls (NUL-separated log).
gh_count() { grep -acz . "$WORK/gh.log" | tr -d ' '; }
# Count recorded `sleep` calls.
sleep_count() { grep -acz . "$WORK/sleep.log" | tr -d ' '; }
# Nth gh call's argv, NUL-separated.
nth_call() { awk -v RS='\0' -v n="$1" 'NR==n {print; exit}' "$WORK/gh.log"; }

# ===========================================================================
# Head's run green passes.
# ===========================================================================
good_run="$(make_run 1 HEAD123 completed success 2026-09-19T11:00:00Z 7 https://x/r/1)"
make_page "$good_run" > "$WORK/green.resp"
out="$(run_script "$WORK/green.resp")"; rc=$?
check "green head exits 0" "0" "$rc"
case "$out" in
	*"concluded success"*) check "green head prints the conclusion" "yes" "yes" ;;
	*) check "green head prints the conclusion" "concluded-success" "$(printf '%s' "$out" | head -1)" ;;
esac
case "$out" in
	*"run #7"*) check "green head names run #7 (the verdict line)" "yes" "yes" ;;
	*) check "green head names run #7 (the verdict line)" "run-7" "$(printf '%s' "$out" | head -1)" ;;
esac
case "$out" in
	*"on main"*) check "green head names the branch" "yes" "yes" ;;
	*) check "green head names the branch" "branch=main" "$(printf '%s' "$out" | head -1)" ;;
esac
case "$out" in
	*"HEAD123"*) check "green head names the commit" "yes" "yes" ;;
	*) check "green head names the commit" "commit=HEAD123" "$(printf '%s' "$out" | head -1)" ;;
esac

# ===========================================================================
# Head's run red fails, naming it.
# ===========================================================================
red_run="$(make_run 2 HEAD123 completed failure 2026-09-19T11:00:00Z 8 https://x/r/2)"
make_page "$red_run" > "$WORK/red.resp"
out="$(run_script "$WORK/red.resp")"; rc=$?
check "red head exits 1" "1" "$rc"
case "$out" in
	*"::error::"*"concluded failure"*"https://x/r/2"*) check "red head errors and names the run" "yes" "yes" ;;
	*) check "red head errors and names the run" "error-concluded-failure-url" "$(printf '%s' "$out" | head -1)" ;;
esac

# ===========================================================================
# Head's run cancelled fails. This is the case the old logic got wrong:
# the pass-over of `cancelled` made a cancelled run for the head read
# as a healthy branch.
# ===========================================================================
cancelled_run="$(make_run 3 HEAD123 completed cancelled 2026-09-19T11:00:00Z 9 https://x/r/3)"
make_page "$cancelled_run" > "$WORK/cancelled.resp"
out="$(run_script "$WORK/cancelled.resp")"; rc=$?
check "cancelled head exits 1" "1" "$rc"
case "$out" in
	*"::error::"*"was cancelled"*"https://x/r/3"*) check "cancelled head errors and names the run" "yes" "yes" ;;
	*) check "cancelled head errors and names the run" "error-was-cancelled-url" "$(printf '%s' "$out" | head -1)" ;;
esac
# The pass-over message ("Passed over N cancelled run(s)") must NOT
# appear: the cancelled-run-for-head path is RED, not silent.
case "$out" in
	*"Passed over"*) check "cancelled head does not say 'Passed over'" "no-passed-over" "$(printf '%s' "$out" | head -1)" ;;
	*) check "cancelled head does not say 'Passed over'" "yes" "yes" ;;
esac

# ===========================================================================
# Head's run still running. The first attempt sees status=in_progress
# and an empty conclusion; the second attempt sees the run completed
# with success. The script waits once and passes.
# ===========================================================================
in_progress_run="$(jq -nc --argjson id 4 '{id:4, head_sha:"HEAD123", status:"in_progress", conclusion:null, created_at:"2026-09-19T11:00:00Z", run_number:4, html_url:"https://x/r/4", event:"push"}')"
done_run="$(make_run 4 HEAD123 completed success 2026-09-19T11:00:00Z 4 https://x/r/4)"
make_page "$in_progress_run" > "$WORK/wait1.resp"
make_page "$done_run" >> "$WORK/wait1.resp"
out="$(run_script "$WORK/wait1.resp")"; rc=$?
check "in_progress then success exits 0" "0" "$rc"
check "in_progress then success: 2 gh calls" "2" "$(gh_count)"
check "in_progress then success: 1 sleep" "1" "$(sleep_count)"
case "$out" in
	*"concluded success"*) check "in_progress then success names the conclusion" "yes" "yes" ;;
	*) check "in_progress then success names the conclusion" "concluded-success" "$(printf '%s' "$out" | head -1)" ;;
esac

# ===========================================================================
# Head's run never appears. With MAX_ATTEMPTS=4 (overridable through
# env so the test takes microseconds rather than minutes), the script
# makes 4 gh calls and 3 sleeps (no sleep after the last attempt), then
# fails with one line naming the commit.
# ===========================================================================
printf '\n' > "$WORK/never.resp"
out="$(MAX_ATTEMPTS=4 SLEEP_SECONDS=15 run_script "$WORK/never.resp")"; rc=$?
check "never-appearing exits 1" "1" "$rc"
check "never-appearing: 4 gh calls" "4" "$(gh_count)"
check "never-appearing: 3 sleeps (one fewer than attempts)" "3" "$(sleep_count)"
case "$out" in
	*"::error::"*"no ci.yml run for commit HEAD123 on main"*) check "never-appearing names the commit on one line" "yes" "yes" ;;
	*) check "never-appearing names the commit on one line" "names-commit-one-line" "$(printf '%s' "$out" | head -1)" ;;
esac

# ===========================================================================
# Older green run for a different commit is NOT the verdict. With
# HEAD_SHA=HEAD123, a page that only carries a green run for OTHER
# must fail; the script must not pick that as the verdict. This is
# the case the old logic got wrong: it took the newest run on the
# branch, so an older green run for any commit answered for the head.
# ===========================================================================
older_other="$(make_run 10 OTHER completed success 2026-09-19T10:00:00Z 5 https://x/r/10)"
make_page "$older_other" > "$WORK/other.resp"
out="$(MAX_ATTEMPTS=2 SLEEP_SECONDS=15 run_script "$WORK/other.resp" HEAD123)"; rc=$?
check "older green for a different commit exits 1" "1" "$rc"
check "older green: 2 gh calls (polled again before giving up)" "2" "$(gh_count)"
check "older green: 1 sleep" "1" "$(sleep_count)"
case "$out" in
	*"no ci.yml run for commit HEAD123 on main"*) check "older green: error names the asked-for HEAD" "yes" "yes" ;;
	*) check "older green: error names the asked-for HEAD" "names-asked-for-head" "$(printf '%s' "$out" | head -1)" ;;
esac
# The older green URL must NOT appear on the verdict line. The script
# never picks it, so neither the success nor the run-URL line can
# carry it.
case "$out" in
	*"https://x/r/10"*) check "older green URL does NOT appear anywhere" "no-r10" "$(printf '%s' "$out" | head -1)" ;;
	*) check "older green URL does NOT appear anywhere" "yes" "yes" ;;
esac

# ===========================================================================
# The inverse: an older green run for the SAME head plus an older
# yellow run for a different commit. The green run wins, because the
# head matches.
# ===========================================================================
older_other_yellow="$(make_run 11 OTHER completed failure 2026-09-19T09:00:00Z 6 https://x/r/11)"
green_same_head="$(make_run 12 HEAD123 completed success 2026-09-19T11:00:00Z 12 https://x/r/12)"
make_page "$older_other_yellow" "$green_same_head" > "$WORK/same.resp"
out="$(run_script "$WORK/same.resp")"; rc=$?
check "older yellow + newer green for head exits 0" "0" "$rc"
# The verdict line is the success message (run number, not URL), so
# check the run number that proves the right run won.
case "$out" in
	*"run #12"*) check "verdict line names run #12 (matching head, not run #6)" "yes" "yes" ;;
	*) check "verdict line names run #12 (matching head, not run #6)" "run-12-not-6" "$(printf '%s' "$out" | head -1)" ;;
esac
case "$out" in
	*"run #6"*) check "verdict line does NOT name run #6 (other head)" "no-r6" "$(printf '%s' "$out" | head -1)" ;;
	*) check "verdict line does NOT name run #6 (other head)" "yes" "yes" ;;
esac

# ===========================================================================
# A pull_request run for the same head_sha must not shadow the push run
# for that head. The defect measured 2026-09-20 on the 5.9.5 release
# push for commit 83315af: a release pull request from develop into
# main creates a pull_request run whose head_sha is develop's head
# and whose branch is develop; under `?branch=develop`, that pull_request
# run sat next to the push run for the same commit, was the newer of
# the two, and the gate read the pull_request run for "branch health
# (develop)". That pull_request run was the release PR itself, and
# its ci.yml concluded failure inside that very run when the release
# broke a gate, so the gate read its own failure, made its own run red,
# and every later reader saw a red the gate created. The fix is that
# the script reads the run the branch triggered (the push run), not a
# pull_request run that merely carries the branch as its head.
#
# The page below carries BOTH a newer pull_request run that failed
# AND an older push run that succeeded, for the same head_sha. The
# push run is the verdict: the script should pass and name the push
# run, not the pull_request run. Today (without the filter) the script
# would fail and name the pull_request run instead.
# ===========================================================================
push_run_for_head="$(make_run 60 HEAD123 completed success 2026-09-19T11:00:00Z 60 https://x/r/60 push)"
pr_run_for_head="$(make_run 62 HEAD123 completed failure 2026-09-19T12:00:00Z 62 https://x/r/62 pull_request)"
make_page "$pr_run_for_head" "$push_run_for_head" > "$WORK/pr_shadow.resp"
out="$(run_script "$WORK/pr_shadow.resp")"; rc=$?
check "newer pull_request failure cannot shadow older push success for head (gate must not read a run the branch did not trigger): exit 0" "0" "$rc"
case "$out" in
	*"run #60"*) check "verdict line names run #60 (the push run, not the pull_request run #62)" "yes" "yes" ;;
	*) check "verdict line names run #60 (the push run, not the pull_request run #62)" "run-60-not-62" "$(printf '%s' "$out" | head -1)" ;;
esac
case "$out" in
	*"run #62"*) check "verdict line does NOT name run #62 (the pull_request run)" "no-r62" "$(printf '%s' "$out" | head -1)" ;;
	*) check "verdict line does NOT name run #62 (the pull_request run)" "yes" "yes" ;;
esac
case "$out" in
	*"https://x/r/62"*) check "pull_request run URL #62 must NOT appear (the gate did not read that run)" "no-r62-url" "$(printf '%s' "$out" | head -1)" ;;
	*) check "pull_request run URL #62 must NOT appear (the gate did not read that run)" "yes" "yes" ;;
esac
# The URL the script must have asked is `event=push`. The stub ignores
# URL filters, so the assertion is on the URL the script BUILT. Reading
# gh call #2 (the runs lookup; gh call #1 is the branches/main lookup
# is not present here because HEAD_SHA is set) and checking it carries
# `event=push` is the per-call proof.
case "$(nth_call 1)" in
	*"event=push"*) check "runs URL carries event=push so the API query itself filters out pull_request runs" "yes" "yes" ;;
	*) check "runs URL carries event=push so the API query itself filters out pull_request runs" "event-push-in-url" "$(nth_call 1)" ;;
esac

# ===========================================================================
# The reverse: a newer push failure plus an older push success for the
# same head_sha still fails. The fix has to read the newest push run
# for the head, not "prefer whichever succeeded". The existing
# "newest-run-by-created_at wins" logic is preserved by the event=push
# filter; this case is the proof that the filter did not turn into a
# "skip red runs" pass-over as a side effect.
# ===========================================================================
newer_push_failure="$(make_run 70 HEAD123 completed failure 2026-09-19T12:00:00Z 70 https://x/r/70 push)"
older_push_success="$(make_run 71 HEAD123 completed success 2026-09-19T11:00:00Z 71 https://x/r/71 push)"
make_page "$newer_push_failure" "$older_push_success" > "$WORK/push_pair.resp"
out="$(run_script "$WORK/push_pair.resp")"; rc=$?
check "newer push failure for head still wins over older push success (filter is not 'prefer succeeded'): exit 1" "1" "$rc"
case "$out" in
	*"::error::"*"concluded failure"*"https://x/r/70"*) check "verdict line names run #70 (the newer push failure, not #71)" "yes" "yes" ;;
	*) check "verdict line names run #70 (the newer push failure, not #71)" "run-70-not-71" "$(printf '%s' "$out" | head -1)" ;;
esac
case "$out" in
	*"https://x/r/71"*) check "older push success URL #71 must NOT appear on the verdict line" "no-r71-url" "$(printf '%s' "$out" | head -1)" ;;
	*) check "older push success URL #71 must NOT appear on the verdict line" "yes" "yes" ;;
esac

# ===========================================================================
# Branch head read from the API on a schedule event. HEAD_SHA is empty
# in the environment, so the first call is `repos/$REPO/branches/$BRANCH`
# and extracts `.commit.sha`. The second call is the runs lookup, which
# then picks the run for the freshly-known head.
# ===========================================================================
schedule_head='{"commit":{"sha":"HEAD456"}}'
schedule_run="$(make_run 20 HEAD456 completed success 2026-09-19T12:00:00Z 20 https://x/r/20)"
make_page "$schedule_run" > "$WORK/schedule.resp"
printf '%s\n' "$schedule_head" > "$WORK/schedule.head"
cat "$WORK/schedule.head" "$WORK/schedule.resp" > "$WORK/schedule.combined"
rm -f "$WORK/gh.log" "$WORK/sleep.log"
: >"$WORK/gh.log"
: >"$WORK/sleep.log"
out="$(PATH="$WORK/bin:$PATH" \
	STUB_LOG="$WORK/gh.log" STUB_RESPONSES="$WORK/schedule.combined" SLEEP_LOG="$WORK/sleep.log" \
	WORKFLOW=ci.yml BRANCH=main REPO=owner/repo HEAD_SHA='' GH_TOKEN=dummy \
	MAX_ATTEMPTS=32 SLEEP_SECONDS=15 \
	bash "$script" 2>&1)"
rc=$?
check "schedule: empty env HEAD_SHA exits 0 once branch head is read" "0" "$rc"
case "$out" in
	*"HEAD456"*) check "schedule: verdict line names the branch head read from API" "yes" "yes" ;;
	*) check "schedule: verdict line names the branch head read from API" "head-456-named" "$(printf '%s' "$out" | head -1)" ;;
esac
check "schedule: 2 gh calls (branch head, then runs)" "2" "$(gh_count)"
case "$(nth_call 1)" in
	*"branches/main"*) check "schedule: first gh call is the branch head lookup" "yes" "yes" ;;
	*) check "schedule: first gh call is the branch head lookup" "first-call-is-branch-head" "$(nth_call 1)" ;;
esac
case "$(nth_call 2)" in
	*"workflows/ci.yml/runs"*"branch=main"*"per_page=30"*) check "schedule: second gh call is the runs lookup with the right filters" "yes" "yes" ;;
	*) check "schedule: second gh call is the runs lookup with the right filters" "runs-url-with-filters" "$(nth_call 2)" ;;
esac

# ===========================================================================
# Skipped run for the head still fails: same shape as `cancelled`, but
# the message names `skipped` so a reader can tell the gate decided not
# to exercise the branch rather than the run being interrupted.
# ===========================================================================
skipped_run="$(make_run 30 HEAD123 completed skipped 2026-09-19T11:00:00Z 30 https://x/r/30)"
make_page "$skipped_run" > "$WORK/skipped.resp"
out="$(run_script "$WORK/skipped.resp")"; rc=$?
check "skipped head exits 1" "1" "$rc"
case "$out" in
	*"::error::"*"was skipped"*"https://x/r/30"*) check "skipped head errors and names the run" "yes" "yes" ;;
	*) check "skipped head errors and names the run" "error-was-skipped-url" "$(printf '%s' "$out" | head -1)" ;;
esac

# ===========================================================================
# Unknown conclusion for the head still fails, naming the value.
# ===========================================================================
made_up_run="$(make_run 31 HEAD123 completed made_up 2026-09-19T11:00:00Z 31 https://x/r/31)"
make_page "$made_up_run" > "$WORK/madeup.resp"
out="$(run_script "$WORK/madeup.resp")"; rc=$?
check "unknown conclusion exits 1" "1" "$rc"
case "$out" in
	*"::error::"*"unexpected conclusion made_up"*"https://x/r/31"*) check "unknown conclusion errors and names the value" "yes" "yes" ;;
	*) check "unknown conclusion errors and names the value" "error-unexpected-made_up-url" "$(printf '%s' "$out" | head -1)" ;;
esac

# ===========================================================================
# The branch parameter actually surfaces in the message. Same input
# run, different branch argument: the message names the branch the
# caller asked about, not whichever the harness happens to use.
# ===========================================================================
dev_run="$(make_run 40 HEAD123 completed success 2026-09-19T11:00:00Z 40 https://x/r/40)"
make_page "$dev_run" > "$WORK/branch.resp"
rm -f "$WORK/gh.log" "$WORK/sleep.log"
: >"$WORK/gh.log"
: >"$WORK/sleep.log"
out_main="$(PATH="$WORK/bin:$PATH" \
	STUB_LOG="$WORK/gh.log" STUB_RESPONSES="$WORK/branch.resp" SLEEP_LOG="$WORK/sleep.log" \
	WORKFLOW=ci.yml BRANCH=main REPO=owner/repo HEAD_SHA=HEAD123 GH_TOKEN=dummy \
	bash "$script" 2>&1)"
case "$out_main" in
	*"on main"*) check "verdict names branch=main" "yes" "yes" ;;
	*) check "verdict names branch=main" "branch=main" "$(printf '%s' "$out_main" | head -1)" ;;
esac
rm -f "$WORK/gh.log" "$WORK/sleep.log"
: >"$WORK/gh.log"
: >"$WORK/sleep.log"
out_dev="$(PATH="$WORK/bin:$PATH" \
	STUB_LOG="$WORK/gh.log" STUB_RESPONSES="$WORK/branch.resp" SLEEP_LOG="$WORK/sleep.log" \
	WORKFLOW=ci.yml BRANCH=develop REPO=owner/repo HEAD_SHA=HEAD123 GH_TOKEN=dummy \
	bash "$script" 2>&1)"
case "$out_dev" in
	*"on develop"*) check "verdict names branch=develop" "yes" "yes" ;;
	*) check "verdict names branch=develop" "branch=develop" "$(printf '%s' "$out_dev" | head -1)" ;;
esac

# ===========================================================================
# `::error::` lines go to stdout; the indented explanation under each one
# goes to stderr. This matches what every other `::error::` site in the
# repository does (`reusable-schedule-freshness.yml`, `reusable-dco.yml`,
# and so on), and is the convention GitHub Actions reads to file an
# annotation.
#
# `run_script` merges the two streams with `2>&1`, so a regression that
# swapped them looked identical to a passing run. This case captures the
# streams separately and names the wrong stream in each assertion so a
# future swap surfaces as a failure that tells you which stream landed
# on the wrong side.
# ===========================================================================
streams_run="$(make_run 50 HEAD123 completed failure 2026-09-19T11:00:00Z 50 https://x/r/50)"
make_page "$streams_run" > "$WORK/streams.resp"
rm -f "$WORK/gh.log" "$WORK/sleep.log"
: >"$WORK/gh.log"
: >"$WORK/sleep.log"
PATH="$WORK/bin:$PATH" \
	STUB_LOG="$WORK/gh.log" STUB_RESPONSES="$WORK/streams.resp" SLEEP_LOG="$WORK/sleep.log" \
	WORKFLOW=ci.yml BRANCH=main REPO=owner/repo HEAD_SHA=HEAD123 GH_TOKEN=dummy \
	bash "$script" >"$WORK/streams-out" 2>"$WORK/streams-err"
rc=$?
check "split streams: red conclusion exits 1" "1" "$rc"
case "$(cat "$WORK/streams-out")" in
	*"::error::"*) check "split streams: ::error:: on stdout (not stderr)" "yes" "yes" ;;
	*) check "split streams: ::error:: on stdout (not stderr)" "stdout" "$(head -n1 "$WORK/streams-out")" ;;
esac
case "$(cat "$WORK/streams-err")" in
	*"https://x/r/50"*) check "split streams: detail URL on stderr (not stdout)" "yes" "yes" ;;
	*) check "split streams: detail URL on stderr (not stdout)" "stderr" "$(head -n1 "$WORK/streams-err")" ;;
esac

# The same rule applies to `::warning::`. The transient `gh api`
# failure path prints one `::warning::` per attempt before the
# polling-bound `::error::` fires; the warning must land on stdout,
# the workflow-command stream, not on stderr, or the GitHub Actions
# runner does not read it as an annotation. This sub-case plants a
# failing `gh` (STUB_EXIT_CODE=1) and MAX_ATTEMPTS=4 so the script
# exits 1 with the polling-bound `::error::` and several `::warning::`
# lines on the captured stdout, no `::warning::` on the captured
# stderr. Asserting on the captured files (not on merged `2>&1`) is
# what surfaces a swap as a failure rather than letting it pass.
printf '\n' > "$WORK/streams-warning.resp"
rm -f "$WORK/gh.log" "$WORK/sleep.log"
: >"$WORK/gh.log"
: >"$WORK/sleep.log"
PATH="$WORK/bin:$PATH" \
	STUB_LOG="$WORK/gh.log" STUB_RESPONSES="$WORK/streams-warning.resp" SLEEP_LOG="$WORK/sleep.log" \
	STUB_EXIT_CODE=1 MAX_ATTEMPTS=4 SLEEP_SECONDS=15 \
	WORKFLOW=ci.yml BRANCH=main REPO=owner/repo HEAD_SHA=HEAD123 GH_TOKEN=dummy \
	bash "$script" >"$WORK/streams-warning-out" 2>"$WORK/streams-warning-err"
rc=$?
check "split streams: transient gh failure exits 1" "1" "$rc"
case "$(cat "$WORK/streams-warning-out")" in
	*"::warning::"*) check "split streams: ::warning:: on stdout (not stderr)" "yes" "yes" ;;
	*) check "split streams: ::warning:: on stdout (not stderr)" "stdout" "$(head -n1 "$WORK/streams-warning-out")" ;;
esac
case "$(cat "$WORK/streams-warning-err")" in
	*"::warning::"*) check "split streams: ::warning:: NOT on stderr" "absent" "present" ;;
	*) check "split streams: ::warning:: NOT on stderr" "absent" "absent" ;;
esac

# ===========================================================================
# The poll loop survives a transient `gh api` failure. A rate limit, a
# 5xx, or any other transient API error must not abort the script
# mid-poll: a retry loop that cannot survive one transient error is a
# retry loop that runs exactly once. The script treats the failed call
# as "no answer this attempt" and keeps polling, with a `::warning::`
# naming the failure so it is visible in the job log.
#
# This case plants a stub `gh` that returns exit 1 on every call and
# runs the script with MAX_ATTEMPTS=4. The expected shape: four gh
# calls, three sleeps (one fewer than attempts), a `::warning::` per
# attempt, and the same "no run finished" error the polling-bound
# exhaustion path prints with a healthy API.
# ===========================================================================
printf '\n' > "$WORK/transient.resp"
rm -f "$WORK/gh.log" "$WORK/sleep.log"
: >"$WORK/gh.log"
: >"$WORK/sleep.log"
out="$(MAX_ATTEMPTS=4 STUB_EXIT_CODE=1 \
	PATH="$WORK/bin:$PATH" \
	STUB_LOG="$WORK/gh.log" STUB_RESPONSES="$WORK/transient.resp" \
	SLEEP_LOG="$WORK/sleep.log" \
	WORKFLOW=ci.yml BRANCH=main REPO=owner/repo HEAD_SHA=HEAD123 GH_TOKEN=dummy \
	SLEEP_SECONDS=15 \
	bash "$script" 2>&1)"
rc=$?
check "transient gh failure: script does not abort mid-poll, exits 1" "1" "$rc"
check "transient gh failure: 4 gh calls (polled through every attempt)" "4" "$(gh_count)"
check "transient gh failure: 3 sleeps (one fewer than attempts)" "3" "$(sleep_count)"
case "$out" in
	*"::warning::"*"gh api"*"failed"*) check "transient gh failure: ::warning:: names the failure on every attempt" "yes" "yes" ;;
	*) check "transient gh failure: ::warning:: names the failure on every attempt" "warning-logged" "$(printf '%s' "$out" | head -3)" ;;
esac
# The empty-answer error still fires: the polling bound was exhausted
# without ever seeing a completed run, so the verdict is "no run
# finished in time", not "we know the verdict". Without the loop
# surviving the transient, the script would have aborted with exit 1
# and no message at all.
case "$out" in
	*"no ci.yml run for commit HEAD123 on main"*) check "transient gh failure: empty-answer error fires after the bound" "yes" "yes" ;;
	*) check "transient gh failure: empty-answer error fires after the bound" "empty-answer-error" "$(printf '%s' "$out" | head -1)" ;;
esac

# ===========================================================================
# Missing env vars are fatal: REPO is now required too.
# ===========================================================================
for v in WORKFLOW BRANCH REPO; do
	rc=0
	rm -f "$WORK/gh.log"
	: >"$WORK/gh.log"
	PATH="$WORK/bin:$PATH" \
		STUB_LOG="$WORK/gh.log" STUB_RESPONSES="$WORK/green.resp" \
		HEAD_SHA=HEAD123 GH_TOKEN=dummy \
		WORKFLOW="$([ "$v" = WORKFLOW ] && echo '' || echo ci.yml)" \
		BRANCH="$([ "$v" = BRANCH ] && echo '' || echo main)" \
		REPO="$([ "$v" = REPO ] && echo '' || echo owner/repo)" \
		bash "$script" </dev/null >/dev/null 2>&1 || rc=$?
	check "missing $v is fatal" "1" "$rc"
done

echo
echo "passed: $pass  failed: $fail"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
