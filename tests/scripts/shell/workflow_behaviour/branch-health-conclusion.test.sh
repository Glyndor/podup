#!/usr/bin/env bash
#
# Behaviour tests for `.github/scripts/checks/check-branch-conclusion.sh`.
#
# The script decides whether a branch is red. A branch is red when a run of
# its suite that has FINISHED failed, and only then. The run it judges is the
# newest one for the branch's current head when that has finished, and
# otherwise the last run that finished in a pass or a failure. A head whose
# suite is queued, running or never started is not a verdict: with the runners
# saturated the suite takes longer than any wait this job can afford, and the
# job used to go red after eight minutes of polling with nothing broken
# (podup, 2026-11-24: "no ci.yml run for commit da14bd1 on develop finished
# within 32 attempts"). It used to read the newest run on the branch with
# `cancelled` skipped, which could answer for a commit nobody had tested
# (measured 2026-09-20), so a run of the head that finished still outranks an
# older green one, and a cancelled or skipped run of the head is red.
#
# The harness plants API answers through a stub `gh` on PATH. Each call to
# `gh` increments an index and returns line N of a planted responses file
# (compact JSON, one response per line). The stub records its argv to a log so
# the URL the step built can be checked for the right filters, and `sleep` is
# a stub that must never be called.
#
# Cases, both ways (the function `design_cases`, which the sabotages at the end
# run again against copies of the script with one rule broken):
#   - the head's run green passes; red, cancelled, skipped, timed out and
#     never started fail, naming it
#   - the head's run still queued or running is NOT red: the last run that
#     finished before it is judged and named, when it is an ancestor of the
#     head, recent, and concluded in a pass or a failure of any kind; one that
#     is not an ancestor, or is older than 48 hours, says nothing; a suite
#     stuck for more than 24 hours is red
#   - a head with NO run is given three hours and is red after them: the
#     success of the commit before it is not its own
#   - a failing `gh` is no answer, and not red
#   - a newer pull_request run for the same head does NOT shadow a push run
#   - develop reads the merge queue's runs for its own branch only
#   - the branch head is read from the API on a schedule event when HEAD_SHA
#     is not set in the environment
#
# Plus a handful of structural assertions (missing env vars, unknown
# conclusion, stream split).
#
# Requires: bash, jq, python3 for some helpers, the script under test.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
script="$HERE/.github/scripts/checks/check-branch-conclusion.sh"
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
make_run() { # <id> <head_sha> <status> <conclusion> <created_at> <run_number> <html_url> [event] [head_branch]
	local event="${8:-push}" branch="${9:-}"
	[ -n "$branch" ] || { [ "$event" = merge_group ] && branch="gh-readonly-queue/develop/pr-$1-$2" || branch=main; }
	jq -nc --argjson id "$1" --arg head "$2" --arg status "$3" --arg conclusion "$4" \
		--arg created "$5" --argjson number "$6" --arg url "$7" --arg event "$event" --arg branch "$branch" \
		'{id:$id, head_sha:$head, status:$status, conclusion:(if $conclusion == "" then null else $conclusion end), created_at:$created, run_number:$number, html_url:$url, event:$event, head_branch:$branch}'
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

# Build a jobs response in the shape `actions/runs/{id}/jobs` returns.
# Each entry is `<name>:<conclusion>`; conclusions like `failure`,
# `cancelled`, `timed_out` enter the filter, while `success` and
# `skipped` do not. The failure-case cases below use this to plant
# exactly the jobs the run had, so the script's own verdict on those
# jobs is what the test exercises.
#
# The `printf '%s' "$(jq -nc ...)"` pair matters: jq writes a trailing
# newline after every result, and a newline left inside the JSON
# (between the last job and the closing `]}`) splits the response
# across two stub lines so the jobs page reads as truncated JSON on
# line 2. `printf '%s' "$(...)"` consumes the trailing newline before
# the comma-or-close is emitted, so the page lands on one stub line.
make_jobs() { # <name>:<conclusion> [<name>:<conclusion> ...]
	local first=1
	printf '{"jobs":['
	for entry in "$@"; do
		local name="${entry%%:*}"
		local conclusion="${entry#*:}"
		if [ "$first" = 1 ]; then
			first=0
		else
			printf ','
		fi
		printf '%s' "$(jq -nc --arg name "$name" --arg conclusion "$conclusion" \
			'{name:$name, conclusion:$conclusion}')"
	done
	printf ']}\n'
}

# Run the script under test against planted responses. Combine
# stdout+stderr in `out`, exit code in `rc`. Each call starts from a
# fresh gh.log and sleep.log so call counts are per-invocation.
# The script reads the page once and never waits, so there is no
# polling bound to shrink.
run_script() { # <responses_file> [head_sha]
	local resp="$1"
	local head="${2:-HEAD123}"
	rm -f "$WORK/gh.log" "$WORK/sleep.log"
	: >"$WORK/gh.log"
	: >"$WORK/sleep.log"
	PATH="$WORK/bin:$PATH" \
		STUB_LOG="$WORK/gh.log" STUB_RESPONSES="$resp" SLEEP_LOG="$WORK/sleep.log" \
		WORKFLOW=ci.yml BRANCH="${BRANCH_UNDER_TEST:-main}" REPO=owner/repo HEAD_SHA="$head" GH_TOKEN=dummy \
		NOW_EPOCH="$NOW" \
		bash "$script" 2>&1
}

# The clock the script reads: the fixtures are dated 2026-09-19, and "now" is 13:00 that day.
NOW="$(date -u -d 2026-09-19T13:00:00Z +%s)"
# What the compare API answers about a commit and the head, and what the commits API says of the head's date.
compare_line() { printf '{"status":"%s"}\n' "$1"; }
commit_line() { printf '{"commit":{"committer":{"date":"%s"}}}\n' "$1"; }

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
# The script now fetches the run's jobs and decides on them. Plant one
# real failed job (not branch-health) so the verdict stays red the way
# it did before the branch-health rule was added.
red_jobs="$(make_jobs 'rust / Test (ubuntu-latest):failure')"
{ make_page "$red_run"; printf '%s' "$red_jobs"; } > "$WORK/red.resp"
out="$(run_script "$WORK/red.resp")"; rc=$?
check "red head exits 1" "1" "$rc"
case "$out" in
	*"::error::"*"concluded failure"*"https://x/r/2"*) check "red head errors and names the run" "yes" "yes" ;;
	*) check "red head errors and names the run" "error-concluded-failure-url" "$(printf '%s' "$out" | head -1)" ;;
esac
case "$out" in
	*"Failed jobs:"*"rust / Test (ubuntu-latest)"*) check "red head names the failed job in the error" "yes" "yes" ;;
	*) check "red head names the failed job in the error" "failed-jobs-named" "$(printf '%s' "$out" | head -3)" ;;
esac
case "$(nth_call 2)" in
	*"actions/runs/2/jobs"*) check "red head: jobs URL carries the run id from the runs lookup" "yes" "yes" ;;
	*) check "red head: jobs URL carries the run id from the runs lookup" "url-with-id" "$(nth_call 2)" ;;
esac

# ===========================================================================
# Three real failed jobs are listed whole, separated by ", ". A join that
# alternated its separators would print "a,b c" and misname the jobs.
# ===========================================================================
three_run="$(make_run 21 HEAD123 completed failure 2026-09-19T11:00:00Z 21 https://x/r/21)"
three_jobs="$(make_jobs 'fmt:failure' 'clippy:failure' 'test:failure')"
{ make_page "$three_run"; printf '%s' "$three_jobs"; } > "$WORK/three.resp"
out="$(run_script "$WORK/three.resp")"; rc=$?
check "three failed jobs exit 1" "1" "$rc"
case "$out" in
	*"Failed jobs: fmt, clippy, test"*) check "three failed jobs are listed with ', '" "yes" "yes" ;;
	*) check "three failed jobs are listed with ', '" "fmt, clippy, test" "$(printf '%s' "$out" | grep 'Failed jobs')" ;;
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
# The cases of the design, in a function so that the sabotages at the end run
# the same ones against a copy of the script with one rule broken.
#
# A branch is red when a finished run of its suite failed, or when its head never
# had a suite at all. A head whose suite is queued or running is not red, and is
# judged by the last run that finished before it, so long as that run is an
# ancestor of the head, is recent, and concluded in a pass or a failure of any
# kind (`timed_out` and `startup_failure` are failures). A head with no run is
# given a short grace to start one and is red after it: the success of the commit
# before it is not its own.
# ===========================================================================
design_cases() {
	local out rc head_running done_ok other_red other_to other_sf head_run_old
	head_running="$(make_run 4 HEAD123 in_progress "" 2026-09-19T12:00:00Z 14 https://x/r/4)"
	done_ok="$(make_run 5 OTHER completed success 2026-09-19T10:00:00Z 13 https://x/r/5)"
	other_red="$(make_run 6 OTHER completed failure 2026-09-19T10:30:00Z 13 https://x/r/6)"
	other_to="$(make_run 7 OTHER completed timed_out 2026-09-19T10:30:00Z 13 https://x/r/7)"
	other_sf="$(make_run 8 OTHER completed startup_failure 2026-09-19T10:30:00Z 13 https://x/r/8)"

	# --- the head is running and an ancestor finished green
	{ make_page "$head_running" "$done_ok"; compare_line ahead; } > "$WORK/d1.resp"
	out="$(run_script "$WORK/d1.resp")"; rc=$?
	check "head running, an ancestor green: passes" "0" "$rc"
	check "head running: the page and one compare, no wait" "2 0" "$(gh_count) $(sleep_count)"
	case "$(nth_call 2)" in *"compare/OTHER...HEAD123"*) check "head running: the compare asks whether the run's commit is behind the head" yes yes ;; *) check "head running: the compare asks whether the run's commit is behind the head" compare-url "$(nth_call 2)" ;; esac
	case "$out" in
		*"::notice::"*"has not finished"*"run #13"*"OTHER"*) check "head running: the notice names the head, the run and the commit judged" yes yes ;;
		*) check "head running: the notice names the head, the run and the commit judged" notice "$(printf '%s' "$out" | head -2)" ;;
	esac
	{ make_page "$head_running" "$done_ok"; compare_line identical; } > "$WORK/d1b.resp"
	run_script "$WORK/d1b.resp" >/dev/null; rc=$?
	check "head running, the run's commit is the head itself (identical): passes" "0" "$rc"

	# --- not an ancestor: the run says nothing about the head
	for rel in diverged behind; do
		{ make_page "$head_running" "$done_ok"; compare_line "$rel"; } > "$WORK/d2.resp"
		out="$(run_script "$WORK/d2.resp")"; rc=$?
		check "head running, the green run's commit is $rel: not its verdict, and not red" "0" "$rc"
		case "$out" in *"no run of an ancestor"*) check "head running, $rel: says no ancestor finished" yes yes ;; *) check "head running, $rel: says no ancestor finished" warning "$out" ;; esac
	done
	{ make_page "$head_running" "$other_red"; compare_line diverged; } > "$WORK/d2b.resp"
	run_script "$WORK/d2b.resp" >/dev/null; rc=$?
	check "head running, a red run of a commit that is not an ancestor: not red either" "0" "$rc"
	# A compare call that fails is unknown, not "not an ancestor": the newest finished run is red and its call fails,
	# and the older green run must NOT answer in its place.
	printf '%s\n%s\n%s\n' "$(make_page "$head_running" "$other_red" "$done_ok")" '' "$(compare_line ahead)" > "$WORK/d2c.resp"
	out="$(run_script "$WORK/d2c.resp")"; rc=$?
	check "head running, the newest finished run is red and its compare call fails: not green" "1" "$rc"
	check "...and no older run is tried in its place: the page and the one compare" "2" "$(gh_count)"
	case "$out" in
		*"::warning::"*"compare/OTHER...HEAD123"*) check "...with a ::warning:: that names the call" yes yes ;;
		*) check "...with a ::warning:: that names the call" warning "$(printf '%s' "$out" | head -3)" ;;
	esac
	case "$out" in *"Unknown is not green"*"::error::"*|*"::error::"*"no verdict"*) check "...and an error that says there is no verdict" yes yes ;; *) check "...and an error that says there is no verdict" error "$out" ;; esac

	# --- an ancestor that failed, in every way a suite can fail
	{ make_page "$head_running" "$other_red"; compare_line ahead; make_jobs 'rust / Test (ubuntu-latest):failure'; } > "$WORK/d3.resp"
	out="$(run_script "$WORK/d3.resp")"; rc=$?
	check "head running, the last finished run failed: red" "1" "$rc"
	case "$out" in *"https://x/r/6"*"Failed jobs"*|*"::error::"*"run #13 concluded failure"*) check "head running, red: names that run" yes yes ;; *) check "head running, red: names that run" named "$out" ;; esac
	{ make_page "$head_running" "$other_to"; compare_line ahead; } > "$WORK/d3b.resp"
	out="$(run_script "$WORK/d3b.resp")"; rc=$?
	check "head running, the last finished run timed out: red" "1" "$rc"
	case "$out" in *"concluded timed_out"*) check "head running, timed out: the error says so" yes yes ;; *) check "head running, timed out: the error says so" timed_out "$out" ;; esac
	{ make_page "$head_running" "$other_sf"; compare_line ahead; } > "$WORK/d3c.resp"
	out="$(run_script "$WORK/d3c.resp")"; rc=$?
	check "head running, the last finished run never started (startup_failure): red" "1" "$rc"
	check "head running, timed out or never started: no jobs lookup (nothing to excuse)" "2" "$(gh_count)"

	# --- cancelled and skipped runs of other commits say nothing
	other_cancelled="$(make_run 9 OTHER completed cancelled 2026-09-19T11:00:00Z 12 https://x/r/9)"
	other_skipped="$(make_run 10 OTHER completed skipped 2026-09-19T11:30:00Z 12 https://x/r/10)"
	make_page "$head_running" "$other_cancelled" "$other_skipped" > "$WORK/d4.resp"
	out="$(run_script "$WORK/d4.resp")"; rc=$?
	check "head running, only cancelled and skipped runs of others: passes, with no verdict" "0" "$rc"
	check "head running, nothing to compare: one call" "1" "$(gh_count)"

	# --- the age bounds
	old_fallback="$(make_run 11 OTHER completed success 2026-09-16T10:00:00Z 9 https://x/r/11)"
	make_page "$head_running" "$old_fallback" > "$WORK/d5.resp"
	out="$(run_script "$WORK/d5.resp")"; rc=$?
	check "head running, the only finished run is older than 48 hours: not used, not red" "0" "$rc"
	check "head running, the stale run is not even compared" "1" "$(gh_count)"
	head_run_old="$(make_run 4 HEAD123 queued "" 2026-09-18T10:00:00Z 14 https://x/r/4)"
	make_page "$head_run_old" "$done_ok" > "$WORK/d6.resp"
	out="$(run_script "$WORK/d6.resp")"; rc=$?
	check "the head's run has been queued for more than 24 hours: red (a stuck suite)" "1" "$rc"
	case "$out" in *"more than 24 hours"*"https://x/r/4"*|*"::error::"*"more than 24 hours"*) check "...and the error says so and names the run" yes yes ;; *) check "...and the error says so and names the run" stuck "$out" ;; esac

	# --- the head's own run finished badly
	for c in timed_out startup_failure; do
		own="$(make_run 20 HEAD123 completed "$c" 2026-09-19T11:00:00Z 20 https://x/r/20)"
		make_page "$own" > "$WORK/d7.resp"
		out="$(run_script "$WORK/d7.resp")"; rc=$?
		check "the head's own run $c: red" "1" "$rc"
	done

	for c in cancelled skipped; do
		own="$(make_run 21 HEAD123 completed "$c" 2026-09-19T11:00:00Z 21 https://x/r/21)"
		make_page "$own" "$done_ok" > "$WORK/d7b.resp"
		run_script "$WORK/d7b.resp" >/dev/null; rc=$?
		check "the head's own run $c, with a green run of an older commit: red" "1" "$rc"
	done

	# --- the head has no run at all
	make_page "$done_ok" > "$WORK/d8.resp"; commit_line 2026-09-19T12:00:00Z >> "$WORK/d8.resp"
	out="$(run_script "$WORK/d8.resp")"; rc=$?
	check "no run for the head, the commit is an hour old: grace, and not red" "0" "$rc"
	check "no run for the head: the page and the date of the commit" "2" "$(gh_count)"
	case "$(nth_call 2)" in *"commits/HEAD123"*) check "no run for the head: the second call reads the commit of the head" yes yes ;; *) check "no run for the head: the second call reads the commit of the head" commits-url "$(nth_call 2)" ;; esac
	case "$out" in *"grace"*) check "no run for the head, inside the grace: the warning says so" yes yes ;; *) check "no run for the head, inside the grace: the warning says so" grace "$out" ;; esac
	make_page "$done_ok" > "$WORK/d9.resp"; commit_line 2026-09-19T08:00:00Z >> "$WORK/d9.resp"
	out="$(run_script "$WORK/d9.resp")"; rc=$?
	check "no run for the head, the commit is five hours old: red, a green run of another commit does not answer for it" "1" "$rc"
	case "$out" in *"::error::head without CI"*"HEAD123"*) check "...and the error says the head has no CI" yes yes ;; *) check "...and the error says the head has no CI" head-without-ci "$out" ;; esac
	{ make_page; commit_line 2026-09-18T08:00:00Z; } > "$WORK/d9b.resp"
	run_script "$WORK/d9b.resp" >/dev/null; rc=$?
	check "no run at all anywhere, an old commit: red" "1" "$rc"
	{ make_page; printf '{"commit":{}}\n'; } > "$WORK/d9c.resp"
	run_script "$WORK/d9c.resp" >/dev/null; rc=$?
	check "no run for the head and the commit date is unreadable: no verdict, not red" "0" "$rc"

	# --- gh itself fails: nothing was learnt, and nothing is red
	printf '\n' > "$WORK/transient.resp"
	rm -f "$WORK/gh.log" "$WORK/sleep.log"; : >"$WORK/gh.log"; : >"$WORK/sleep.log"
	out="$(STUB_EXIT_CODE=1 PATH="$WORK/bin:$PATH" STUB_LOG="$WORK/gh.log" STUB_RESPONSES="$WORK/transient.resp" SLEEP_LOG="$WORK/sleep.log" \
		WORKFLOW=ci.yml BRANCH="${BRANCH_UNDER_TEST:-main}" REPO=owner/repo HEAD_SHA=HEAD123 GH_TOKEN=dummy NOW_EPOCH="$NOW" bash "$script" 2>&1)"; rc=$?
	check "a failing gh: the script survives it and is not red" "0" "$rc"
	check "a failing gh: no sleep" "0" "$(sleep_count)"
	case "$out" in
		*"::warning::"*"gh api call failed"*"runs?branch=main"*) check "a failing gh: ::warning:: names the call that failed" yes yes ;;
		*) check "a failing gh: ::warning:: names the call that failed" warning "$(printf '%s' "$out" | head -3)" ;;
	esac

	# --- develop: the queue's runs, ancestors only, of its own branch only
	local empty_page q_ok q_bad q_other q_main q_pr q_running q_anc
	empty_page="$(make_page)"
	q_ok="$(make_run 81 HEAD123 completed success 2026-09-19T11:00:00Z 31 https://x/r/81 merge_group)"
	q_bad="$(make_run 82 HEAD123 completed failure 2026-09-19T11:05:00Z 32 https://x/r/82 merge_group)"
	q_other="$(make_run 83 OTHER completed failure 2026-09-19T11:06:00Z 33 https://x/r/83 merge_group)"
	q_anc="$(make_run 85 OTHER completed success 2026-09-19T11:06:00Z 35 https://x/r/85 merge_group)"
	q_main="$(make_run 86 HEAD123 completed failure 2026-09-19T11:08:00Z 36 https://x/r/86 merge_group gh-readonly-queue/main/pr-9-abc)"
	q_pr="$(make_run 84 HEAD123 completed failure 2026-09-19T11:07:00Z 34 https://x/r/84 pull_request)"
	q_running="$(make_run 87 HEAD123 queued "" 2026-09-19T11:09:00Z 37 https://x/r/87 merge_group)"
	printf '%s\n%s\n' "$empty_page" "$(make_page "$q_ok")" > "$WORK/q_ok.resp"
	out="$(BRANCH_UNDER_TEST=develop run_script "$WORK/q_ok.resp")"; rc=$?
	check "develop: the queue's green run for the head passes" "0" "$rc"
	case "$out" in *"run #31"*) check "develop: the verdict names the queue's run" yes yes ;; *) check "develop: the verdict names the queue's run" run-31 "$out" ;; esac
	check "develop: the second call asks for the runs of the merge queue, and it is made even when the push page has runs" "1 2" \
		"$(nth_call 2 | grep -c 'event=merge_group&per_page=30') $(gh_count)"
	printf '%s\n%s\n' "$empty_page" "$(make_page "$q_bad")" > "$WORK/q_bad.resp"
	BRANCH_UNDER_TEST=develop run_script "$WORK/q_bad.resp" >/dev/null; rc=$?
	check "develop: the queue's red run for the head fails" "1" "$rc"
	# The head is queued, and a run of the queue for an ancestor failed: red. For a commit that is not an ancestor, not red.
	{ printf '%s\n' "$empty_page"; make_page "$q_running" "$q_other" "$q_pr" "$q_main"; compare_line ahead; make_jobs 'rust / Test (ubuntu-latest):failure'; } > "$WORK/q_anc.resp"
	out="$(BRANCH_UNDER_TEST=develop run_script "$WORK/q_anc.resp")"; rc=$?
	check "develop: the head is queued and the queue's run of an ancestor failed: red" "1" "$rc"
	case "$out" in *"run #33"*) check "develop: and the error names that run, not the pull request's or main's" yes yes ;; *) check "develop: and the error names that run, not the pull request's or main's" run-33 "$out" ;; esac
	{ printf '%s\n' "$empty_page"; make_page "$q_running" "$q_other"; compare_line diverged; } > "$WORK/q_nonanc.resp"
	BRANCH_UNDER_TEST=develop run_script "$WORK/q_nonanc.resp" >/dev/null; rc=$?
	check "develop: the head is queued and the failed queue run is not an ancestor of it: not red" "0" "$rc"
	{ printf '%s\n' "$empty_page"; make_page "$q_running" "$q_anc"; compare_line ahead; } > "$WORK/q_pend.resp"
	out="$(BRANCH_UNDER_TEST=develop run_script "$WORK/q_pend.resp")"; rc=$?
	check "develop: the head's queued run and the green run of an ancestor: passes" "0" "$rc"
	case "$out" in *"has not finished"*"run #35"*) check "develop: and the notice names the run judged" yes yes ;; *) check "develop: and the notice names the run judged" notice "$out" ;; esac
	# A pull request run and the queue of another branch are no run of the head at all.
	{ printf '%s\n' "$empty_page"; make_page "$q_pr" "$q_main"; commit_line 2026-09-19T08:00:00Z; } > "$WORK/q_nothing.resp"
	out="$(BRANCH_UNDER_TEST=develop run_script "$WORK/q_nothing.resp")"; rc=$?
	check "develop: only a pull request run and the queue of main, and an old commit: the head has no CI, red" "1" "$rc"
	{ printf '%s\n' "$empty_page"; printf '%s\n' "$empty_page"; commit_line 2026-09-19T12:30:00Z; } > "$WORK/q_none.resp"
	BRANCH_UNDER_TEST=develop run_script "$WORK/q_none.resp" >/dev/null; rc=$?
	check "develop: nothing at all and a commit of half an hour: grace, not red" "0" "$rc"
	{ printf '%s\n' "$empty_page"; commit_line 2026-09-19T12:30:00Z; } > "$WORK/m_none.resp"
	BRANCH_UNDER_TEST=main run_script "$WORK/m_none.resp" >/dev/null; rc=$?
	check "main asks for its push run and then for the commit: two calls, no queue lookup" "2" "$(gh_count)"
	check "main with no run and a fresh commit passes with a warning" "0" "$rc"
}

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
# Same shape as the red-head case: plant a real failed job so the
# verdict stays red after the branch-health rule is applied.
push_pair_jobs="$(make_jobs 'rust / Test (ubuntu-latest):failure')"
{ make_page "$newer_push_failure" "$older_push_success"; printf '%s' "$push_pair_jobs"; } > "$WORK/push_pair.resp"
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
	WORKFLOW=ci.yml BRANCH="${BRANCH_UNDER_TEST:-main}" REPO=owner/repo HEAD_SHA='' GH_TOKEN=dummy \
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
	WORKFLOW=ci.yml BRANCH="${BRANCH_UNDER_TEST:-main}" REPO=owner/repo HEAD_SHA=HEAD123 GH_TOKEN=dummy \
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
streams_jobs="$(make_jobs 'rust / Test (ubuntu-latest):failure')"
{ make_page "$streams_run"; printf '%s' "$streams_jobs"; } > "$WORK/streams.resp"
rm -f "$WORK/gh.log" "$WORK/sleep.log"
: >"$WORK/gh.log"
: >"$WORK/sleep.log"
PATH="$WORK/bin:$PATH" \
	STUB_LOG="$WORK/gh.log" STUB_RESPONSES="$WORK/streams.resp" SLEEP_LOG="$WORK/sleep.log" \
	WORKFLOW=ci.yml BRANCH="${BRANCH_UNDER_TEST:-main}" REPO=owner/repo HEAD_SHA=HEAD123 GH_TOKEN=dummy \
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

# The same rule applies to `::warning::`. A `gh api` that fails is "no answer",
# and the warning that names it must land on stdout, the workflow-command stream,
# or the GitHub Actions runner does not read it as an annotation. This sub-case
# plants a failing `gh` (STUB_EXIT_CODE=1): the script passes, because nothing was
# learnt about the branch, with `::warning::` lines on the captured stdout and none
# on the captured stderr. Asserting on the captured files (not on merged `2>&1`)
# is what surfaces a swap as a failure rather than letting it pass.
printf '\n' > "$WORK/streams-warning.resp"
rm -f "$WORK/gh.log" "$WORK/sleep.log"
: >"$WORK/gh.log"
: >"$WORK/sleep.log"
PATH="$WORK/bin:$PATH" \
	STUB_LOG="$WORK/gh.log" STUB_RESPONSES="$WORK/streams-warning.resp" SLEEP_LOG="$WORK/sleep.log" \
	STUB_EXIT_CODE=1 \
	WORKFLOW=ci.yml BRANCH="${BRANCH_UNDER_TEST:-main}" REPO=owner/repo HEAD_SHA=HEAD123 GH_TOKEN=dummy \
	bash "$script" >"$WORK/streams-warning-out" 2>"$WORK/streams-warning-err"
rc=$?
check "split streams: transient gh failure is no verdict, and not red" "0" "$rc"
case "$(cat "$WORK/streams-warning-out")" in
	*"::warning::"*) check "split streams: ::warning:: on stdout (not stderr)" "yes" "yes" ;;
	*) check "split streams: ::warning:: on stdout (not stderr)" "stdout" "$(head -n1 "$WORK/streams-warning-out")" ;;
esac
case "$(cat "$WORK/streams-warning-err")" in
	*"::warning::"*) check "split streams: ::warning:: NOT on stderr" "absent" "present" ;;
	*) check "split streams: ::warning:: NOT on stderr" "absent" "absent" ;;
esac

# ===========================================================================
# A `failure` run whose only non-success job is `branch health (main) /
# branch health` is the latch the rule at the top of this file describes:
# the run is red only because the OTHER branch's health gate decided it
# was, and the branch's own code passed. The script must exit 0 with a
# single line that names the branch-health job so an operator can tell at
# a glance that the run was green for everything except the gate that
# handed the latch back. Without this case the rule on production would
# silently keep both long-lived branches red on every push.
# ===========================================================================
bh_only_run="$(make_run 80 HEAD123 completed failure 2026-09-19T11:00:00Z 80 https://x/r/80)"
bh_only_jobs="$(make_jobs 'branch health (main) / branch health:failure')"
{ make_page "$bh_only_run"; printf '%s' "$bh_only_jobs"; } > "$WORK/bh_only.resp"
out="$(run_script "$WORK/bh_only.resp")"; rc=$?
check "branch-health-only failure exits 0 (the latch, not the branch)" "0" "$rc"
case "$out" in
	*"concluded failure only on branch-health"*"branch health (main) / branch health"*) check "branch-health-only failure names the branch-health job in one line" "yes" "yes" ;;
	*) check "branch-health-only failure names the branch-health job in one line" "named-bh-only" "$(printf '%s' "$out" | head -1)" ;;
esac
case "$out" in
	*"::error::"*) check "branch-health-only failure does NOT emit ::error:: (it is green)" "no-error" "error" ;;
	*) check "branch-health-only failure does NOT emit ::error:: (it is green)" "yes" "yes" ;;
esac
case "$(nth_call 2)" in
	*"actions/runs/80/jobs"*) check "branch-health-only: jobs URL carries the run id 80" "yes" "yes" ;;
	*) check "branch-health-only: jobs URL carries the run id 80" "url-with-id-80" "$(nth_call 2)" ;;
esac

# ===========================================================================
# The inverse: a `failure` run with the branch-health failure AND a real
# failed job (e.g. `rust / Test (ubuntu-latest)`). The script must stay
# red and name the real job. The branch-health job, having been the
# visible face of the latch, must NOT be the job the error names: the
# operator needs the real failure to be the first thing they see, and
# mixing the latch into the same line buries it.
# ===========================================================================
bh_plus_real_run="$(make_run 81 HEAD123 completed failure 2026-09-19T11:00:00Z 81 https://x/r/81)"
bh_plus_real_jobs="$(make_jobs 'branch health (main) / branch health:failure' 'rust / Test (ubuntu-latest):failure')"
{ make_page "$bh_plus_real_run"; printf '%s' "$bh_plus_real_jobs"; } > "$WORK/bh_plus_real.resp"
out="$(run_script "$WORK/bh_plus_real.resp")"; rc=$?
check "branch-health + real failure exits 1 (the real failure wins)" "1" "$rc"
case "$out" in
	*"::error::"*"concluded failure"*"https://x/r/81"*) check "branch-health + real failure errors and names the run" "yes" "yes" ;;
	*) check "branch-health + real failure errors and names the run" "error-concluded-failure-url" "$(printf '%s' "$out" | head -1)" ;;
esac
case "$out" in
	*"Failed jobs:"*"rust / Test (ubuntu-latest)"*) check "branch-health + real failure names the real job on the 'Failed jobs:' line" "yes" "yes" ;;
	*) check "branch-health + real failure names the real job on the 'Failed jobs:' line" "real-named" "$(printf '%s' "$out" | tail -3)" ;;
esac
# The 'Failed jobs:' line lists the real job; branch-health must not
# appear there because the script filters it out before printing the
# verdict line.
case "$out" in
	*"Failed jobs:"*"branch health"*) check "branch-health + real failure does NOT list branch health as a failed job" "no" "yes" ;;
	*) check "branch-health + real failure does NOT list branch health as a failed job" "yes" "yes" ;;
esac
case "$(nth_call 2)" in
	*"actions/runs/81/jobs"*) check "branch-health + real failure: jobs URL carries the run id 81" "yes" "yes" ;;
	*) check "branch-health + real failure: jobs URL carries the run id 81" "url-with-id-81" "$(nth_call 2)" ;;
esac

# ===========================================================================
# A `failure` run whose jobs call cannot be answered. The polling loop
# above treats a transient `gh api` failure as "no answer this attempt"
# and the failure case here must do the same: an unanswered question is
# not a pass. The verdict stays red with a message that says the jobs
# could not be seen, so an operator reading the error knows the gate
# could not rule the latch out.
#
# The responses file has only one line (the runs page). The stub hands
# out line N on call N, so the second call (the jobs lookup) gets an
# empty answer, which jq parses as an error and the stub reports as a
# non-zero exit. That is what the script's `if jobs=...` sees.
# ===========================================================================
bh_call_fail_run="$(make_run 82 HEAD123 completed failure 2026-09-19T11:00:00Z 82 https://x/r/82)"
make_page "$bh_call_fail_run" > "$WORK/bh_call_fail.resp"
out="$(run_script "$WORK/bh_call_fail.resp")"; rc=$?
check "jobs fetch failure: exits 1 (an unknown verdict is not green)" "1" "$rc"
case "$out" in
	*"::error::"*"concluded failure"*"https://x/r/82"*) check "jobs fetch failure: errors and names the run" "yes" "yes" ;;
	*) check "jobs fetch failure: errors and names the run" "error-concluded-failure-url" "$(printf '%s' "$out" | head -1)" ;;
esac
case "$out" in
	*"Could not"*"jobs"*) check "jobs fetch failure: explains the unknown verdict" "yes" "yes" ;;
	*) check "jobs fetch failure: explains the unknown verdict" "explains-unknown" "$(printf '%s' "$out" | head -3)" ;;
esac
case "$(nth_call 2)" in
	*"actions/runs/82/jobs"*) check "jobs fetch failure: jobs URL still carries the run id 82 (the call was attempted)" "yes" "yes" ;;
	*) check "jobs fetch failure: jobs URL still carries the run id 82 (the call was attempted)" "url-with-id-82" "$(nth_call 2)" ;;
esac

# ===========================================================================
# A `cancelled` run is the run's own verdict; the latch rule does not
# apply to it. Even when the only job in the run is a branch-health
# one, the cancellation is the verdict and the gate must stay red. The
# existing cancellation tests pin this; this case is the explicit one
# that names branch health, so a future change that quietly adds a
# jobs lookup to the cancelled branch surfaces here.
# ===========================================================================
cancelled_bh_run="$(make_run 83 HEAD123 completed cancelled 2026-09-19T11:00:00Z 83 https://x/r/83)"
make_page "$cancelled_bh_run" > "$WORK/cancelled_bh.resp"
out="$(run_script "$WORK/cancelled_bh.resp")"; rc=$?
check "cancelled run with only branch health: still exits 1 (cancellation is the verdict)" "1" "$rc"
case "$out" in
	*"::error::"*"was cancelled"*"https://x/r/83"*) check "cancelled + branch health: errors and names the run as cancelled" "yes" "yes" ;;
	*) check "cancelled + branch health: errors and names the run as cancelled" "error-was-cancelled-url" "$(printf '%s' "$out" | head -1)" ;;
esac
# No jobs lookup must have happened: cancelled is not a failure and the
# failure branch is the only one that consults the run's jobs.
check "cancelled + branch health: only 1 gh call (no jobs lookup)" "1" "$(gh_count)"

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

design_cases

# ===========================================================================
# The sabotages: a copy of the script with one rule broken, run against the
# cases of the design, must fail at least one of them. A rule no case reads is
# a rule that can be dropped without anybody noticing.
# ===========================================================================
sabotage() { # <description> <text to find, once> <text to put there>
	local copy="$WORK/sabotaged-$RANDOM.sh" real="$script" before_fail="$fail" before_pass="$pass"
	if ! python3 - "$real" "$copy" "$2" "$3" <<'PYEOF'
import sys
src, dst, old, new = sys.argv[1:5]
text = open(src).read()
if text.count(old) != 1:
    sys.exit(3)
open(dst, "w").write(text.replace(old, new))
PYEOF
	then
		check "the sabotage applies: $1" applied not-applied
		return
	fi
	chmod +x "$copy"
	script="$copy"
	design_cases >/dev/null 2>&1
	script="$real"
	local killed=$((fail - before_fail))
	fail="$before_fail"
	pass="$before_pass"
	check "sabotage: $1" yes "$([ "$killed" -gt 0 ] && echo yes || echo no)"
}
# shellcheck disable=SC2016 # the texts are literal script text
sabotage "a failed compare call is read as not an ancestor, and the next older run answers" 'if ! rel="$(gh api "repos/${repo}/compare/${csha}...${head_sha}" --jq '"'"'.status'"'"' 2>/dev/null)" || [ -z "$rel" ]; then' 'if ! rel="$(gh api "repos/${repo}/compare/${csha}...${head_sha}" --jq '"'"'.status'"'"' 2>/dev/null)" && false; then'
sabotage "a head with no run is never red" 'if [ "$age" -gt $((grace_hours * 3600)) ]; then' 'if false; then'
sabotage "a run that is not an ancestor is trusted" 'ahead | identical)' 'ahead | identical | diverged | behind | "")'
sabotage "a timed out run is not a failure" ' or .conclusion == \"timed_out\"' ''
sabotage "a run that never started is not a failure" ' or .conclusion == \"startup_failure\"' ''
sabotage "a stuck suite is not red" 'if [ "$age" -gt $((pending_hours * 3600)) ]; then' 'if false; then'
sabotage "a finished run of any age is trusted" 'fromdateiso8601) >= \$cutoff' 'fromdateiso8601) >= 0'
sabotage "the head's own timed out run is not red" '	timed_out | startup_failure)' '	never_a_conclusion)'
sabotage "the fallback also fires when the head has no run" 'elif [ -n "$mine_pending" ]; then' 'elif true; then'
sabotage "the head's own cancelled run is no longer red" '		echo "  and the next tick will read the new run." >&2
		exit 1' '		echo "  and the next tick will read the new run." >&2
		exit 0'

echo
echo "passed: $pass  failed: $fail"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
