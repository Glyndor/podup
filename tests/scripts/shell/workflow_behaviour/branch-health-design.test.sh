#!/usr/bin/env bash
#
# The design of `.github/scripts/checks/check-branch-conclusion.sh`, case by case and rule by
# rule: when a branch is red, and when it is not.
#
# A branch is red when a finished run of its suite failed (of any kind: failure, timed_out,
# startup_failure), or when its head never had a suite at all. A head whose suite is queued or
# running is NOT red: it is judged by the last run that finished before it, if that run is an
# ancestor of the head, is recent, and concluded in a pass or a failure, and a compare call that
# fails is unknown and red, never a reason to try an older run. A head with no run is given a
# grace counted from when the SERVER recorded its arrival, and is red after it; a time that is
# missing, unreadable or in the future is red as well. The cases are the function
# `design_cases`; the sabotages at the end run the same cases against a copy of the script with
# one rule broken, and each must turn at least one of them red.
#
# The harness is the one of branch-health-conclusion.test.sh: a stub `gh` that hands out line N of
# a planted responses file on call N, applying `--jq` with real jq, and a stub `sleep` that must
# never be called.
#
# Requires: bash, jq, python3.
# shellcheck disable=SC2016 # the sabotages and the greps hold literal script text
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
# What the server recorded about the branch: one entry, the push that brought the head.
activity_line() { printf '[{"after":"HEAD123","timestamp":"%s","activity_type":"push"}]\n' "$1"; }

# Count recorded `gh` calls (NUL-separated log).
gh_count() { grep -acz . "$WORK/gh.log" | tr -d ' '; }
# Count recorded `sleep` calls.
sleep_count() { grep -acz . "$WORK/sleep.log" | tr -d ' '; }
# Nth gh call's argv, NUL-separated.
nth_call() { awk -v RS='\0' -v n="$1" 'NR==n {print; exit}' "$WORK/gh.log"; }


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
	make_page "$done_ok" > "$WORK/d8.resp"; activity_line 2026-09-19T12:00:00Z >> "$WORK/d8.resp"
	out="$(run_script "$WORK/d8.resp")"; rc=$?
	check "no run for the head, the commit is an hour old: grace, and not red" "0" "$rc"
	check "no run for the head: the page and the date of the commit" "2" "$(gh_count)"
	case "$(nth_call 2)" in *"activity?ref=refs%2Fheads%2Fmain"*) check "no run for the head: the second call reads when the server recorded the head arrived" yes yes ;; *) check "no run for the head: the second call reads when the server recorded the head arrived" activity-url "$(nth_call 2)" ;; esac
	case "$out" in *"grace"*) check "no run for the head, inside the grace: the warning says so" yes yes ;; *) check "no run for the head, inside the grace: the warning says so" grace "$out" ;; esac
	make_page "$done_ok" > "$WORK/d9.resp"; activity_line 2026-09-19T08:00:00Z >> "$WORK/d9.resp"
	out="$(run_script "$WORK/d9.resp")"; rc=$?
	check "no run for the head, the commit is five hours old: red, a green run of another commit does not answer for it" "1" "$rc"
	case "$out" in *"::error::head without CI"*"HEAD123"*) check "...and the error says the head has no CI" yes yes ;; *) check "...and the error says the head has no CI" head-without-ci "$out" ;; esac
	{ make_page; activity_line 2026-09-18T08:00:00Z; } > "$WORK/d9b.resp"
	run_script "$WORK/d9b.resp" >/dev/null; rc=$?
	check "no run at all anywhere, an old commit: red" "1" "$rc"
	# A time that cannot be believed is red, never "inside the grace": the author of a commit writes its dates,
	# and the server's record may be missing.
	make_page > "$WORK/d9c.page"
	for bad in "2026-09-20T13:00:00Z|a time in the future" "2026-09-19T13:04:00Z|a time a few minutes ahead of the clock, beyond no skew" "|an empty time" "not-a-date|an unreadable time"; do
		ts="${bad%%|*}"; what="${bad#*|}"
		{ cat "$WORK/d9c.page"; printf '[{"after":"HEAD123","timestamp":"%s"}]\n' "$ts"; } > "$WORK/d9c.resp"
		case "$ts" in 2026-09-19T13:04:00Z) continue ;; esac
		out="$(run_script "$WORK/d9c.resp")"; rc=$?
		check "no run for the head and $what: red, not the grace" "1" "$rc"
	done
	# Seconds of difference between the two clocks are not a future.
	{ cat "$WORK/d9c.page"; activity_line 2026-09-19T13:01:00Z; } > "$WORK/d9d.resp"
	run_script "$WORK/d9d.resp" >/dev/null; rc=$?
	check "no run for the head and a time a minute ahead of the clock (skew): inside the grace" "0" "$rc"
	{ cat "$WORK/d9c.page"; printf '[{"after":"OTHER","timestamp":"2026-09-19T12:30:00Z"}]\n'; } > "$WORK/d9e.resp"
	out="$(run_script "$WORK/d9e.resp")"; rc=$?
	check "no run for the head and no activity entry for it: red" "1" "$rc"
	{ cat "$WORK/d9c.page"; printf '[]\n'; } > "$WORK/d9f.resp"
	run_script "$WORK/d9f.resp" >/dev/null; rc=$?
	check "no run for the head and an empty activity: red" "1" "$rc"
	{ cat "$WORK/d9c.page"; printf '\n'; } > "$WORK/d9g.resp"
	out="$(run_script "$WORK/d9g.resp")"; rc=$?
	check "no run for the head and the activity call answers nothing: red" "1" "$rc"
	case "$out" in *"::warning::"*"activity?ref=refs/heads/main"*) check "...with a ::warning:: that names the call" yes yes ;; *) check "...with a ::warning:: that names the call" warning "$out" ;; esac

	# --- gh itself fails: nothing was learnt, and nothing is red
	printf '\n' > "$WORK/transient.resp"
	rm -f "$WORK/gh.log" "$WORK/sleep.log"; : >"$WORK/gh.log"; : >"$WORK/sleep.log"
	out="$(STUB_EXIT_CODE=1 PATH="$WORK/bin:$PATH" STUB_LOG="$WORK/gh.log" STUB_RESPONSES="$WORK/transient.resp" SLEEP_LOG="$WORK/sleep.log" \
		WORKFLOW=ci.yml BRANCH="${BRANCH_UNDER_TEST:-main}" REPO=owner/repo HEAD_SHA=HEAD123 GH_TOKEN=dummy NOW_EPOCH="$NOW" bash "$script" 2>&1)"; rc=$?
	check "a failing gh: the script survives it, and with no run and no time it is red: unknown is not green" "1" "$rc"
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
	{ printf '%s\n' "$empty_page"; make_page "$q_pr" "$q_main"; activity_line 2026-09-19T08:00:00Z; } > "$WORK/q_nothing.resp"
	out="$(BRANCH_UNDER_TEST=develop run_script "$WORK/q_nothing.resp")"; rc=$?
	check "develop: only a pull request run and the queue of main, and an old commit: the head has no CI, red" "1" "$rc"
	{ printf '%s\n' "$empty_page"; printf '%s\n' "$empty_page"; activity_line 2026-09-19T12:30:00Z; } > "$WORK/q_none.resp"
	BRANCH_UNDER_TEST=develop run_script "$WORK/q_none.resp" >/dev/null; rc=$?
	check "develop: nothing at all and a commit of half an hour: grace, not red" "0" "$rc"
	{ printf '%s\n' "$empty_page"; activity_line 2026-09-19T12:30:00Z; } > "$WORK/m_none.resp"
	BRANCH_UNDER_TEST=main run_script "$WORK/m_none.resp" >/dev/null; rc=$?
	check "main asks for its push run and then for the commit: two calls, no queue lookup" "2" "$(gh_count)"
	check "main with no run and a fresh commit passes with a warning" "0" "$rc"
}

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
sabotage "a failed compare call is read as not an ancestor, and the next older run answers" 'if ! rel="$(gh api "repos/${repo}/compare/${csha}...${head_sha}" --jq '"'"'.status'"'"' 2>/dev/null)" || [ -z "$rel" ]; then' 'if ! rel="$(gh api "repos/${repo}/compare/${csha}...${head_sha}" --jq '"'"'.status'"'"' 2>/dev/null)" && false; then'
sabotage "a head with no run is never red" 'if [ "$age" -gt $((grace_hours * 3600)) ]; then' 'if false; then'
sabotage "a time in the future is believed" 'if [ "$age" -lt -300 ]; then' 'if false; then'
sabotage "an unreadable time is read as inside the grace" 'arrived_s="$(date -d "$arrived" +%s 2>/dev/null)" || no_time "the time '"'"'${arrived}'"'"' is not a date"' 'arrived_s="$(date -d "$arrived" +%s 2>/dev/null)" || arrived_s="$now"'
sabotage "an empty time is read as inside the grace" '[ -n "$arrived" ] || no_time "the activity of the branch has no entry for the head, or it has no time"' '[ -n "$arrived" ] || arrived="$(date -u -d @"$now" +%Y-%m-%dT%H:%M:%SZ)"'
sabotage "an activity call that fails is read as inside the grace" '		no_time "the activity of the branch could not be read"' '		exit 0'
sabotage "the grace is counted from a time the author of the commit writes" "arrived=\"\$(jq -r --arg head \"\$head_sha\" '[.[]? | select(.after == \$head)][0].timestamp // \"\"' <<<\"\$activity\" 2>/dev/null)\" || arrived=\"\"" 'arrived="$(date -u -d @"$((now - 60))" +%Y-%m-%dT%H:%M:%SZ)"'
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
