#!/usr/bin/env bash
#
# Cut the runs of a pull request or of a merge group at the FIRST failed job, and not when the last one
# has finished.
#
# `cancel-others` runs after a workflow has failed as a whole, which is after its slowest job: a job
# that fails at minute 10 left the rest of the run going for the 40 more minutes it needed, on
# runners that are scarce and with a queue behind them. This is a job that is there from the start,
# looks at the check runs of the commit, and when one of them has failed it cancels the runs that are
# still going.
#
# Called by the `watch` job of reusable-stop-on-failure.yml, which holds `actions: write`, `checks: read`
# and `contents: read` and nothing else, and which runs the copy of this script that the BASE commit holds.
# Nothing here is a third-party action: it is `gh api` with the token of the run.
#
# Inputs (env), all required except the three times, and none of them can widen what is cancelled:
#   GH_TOKEN, REPO, RUN_ID (the run of the watcher), HEAD_SHA (the commit whose checks are watched: the head
#   of the pull request, or the head of the merge group), EVENT (`pull_request` or `merge_group`), HEAD_BRANCH
#   (the branch of the pull request, or of the merge group without `refs/heads/`) and, on a pull request,
#   PR_NUMBER.
#   INTERVAL (seconds between two looks, 30), WARMUP (seconds before "nothing is left" counts, 240) and LIMIT
#   (seconds the job may watch, 7000) only say how long it looks.
#
# Whose a run is. Two pull requests, a fork, a merge group and a schedule can share a commit, and the check
# runs of a commit say nothing about which of them they belong to. A run is this one's only when it is of this
# repository (head_repository), of this event, of this commit, of this branch, and on a pull request of this
# number. A failed check counts only if its run is one of those, and only those runs are cancelled.
#
# One look is ONE request: the check runs of the commit, which carry every job of every workflow with its
# status, its conclusion and the address of its run. The token of a repository has 1000 requests an hour to
# spend, and a look every 30 seconds is 120. The list of runs is read only when a failed check appears whose
# run is not yet known to be this one's, and once for each such run.
#
# What it does, and only that:
#   - a check run of GitHub Actions that concluded `failure` or `timed_out`, of a run that is this one's, is a
#     failure; `cancelled`, `skipped`, `neutral` and a job allowed to fail (it concludes `success`) are not;
#   - on a failure it cancels the runs of this pull request or merge group that have not finished, whose workflow
#     file is in CANCELLABLE, and last its own run;
#   - it never cancels a security analysis, a release, a publication, a run of another commit, branch, event
#     or repository: those are not in the list or not in the set, and the test fails if a workflow whose name
#     says it is one is added to the list;
#   - the aggregators of the workflows it cancels keep `if: always()`, so they run in a cancelled run and
#     publish red under their own names: a cut leaves the required checks red, never absent or green;
#   - it never fails. A look that cannot be read, a cancellation that is refused (a read-only token, as for
#     Dependabot and forks) and a missing input are reported and the job still succeeds.
set -uo pipefail
export LC_ALL=C

# The workflows a failure may cut: the expensive and functional ones, the same as cancel-others.sh. The list is
# this one: no environment variable adds to it.
CANCELLABLE=(
	.github/workflows/ci.yml
	.github/workflows/podman-lane.yml
	.github/workflows/debian-build.yml
	.github/workflows/binary-budget.yml
	.github/workflows/asset-contract.yml
	.github/workflows/lint-shell.yml
	.github/workflows/lint-powershell.yml
	.github/workflows/linux-hardening.yml
	.github/workflows/windows-arm64.yml
	.github/workflows/fuzz.yml
)

repo="${REPO:-}"
own="${RUN_ID:-}"
sha="${HEAD_SHA:-}"
event="${EVENT:-}"
branch="${HEAD_BRANCH:-}"
pr="${PR_NUMBER:-}"
interval="${INTERVAL:-30}"
warmup="${WARMUP:-240}"
limit="${LIMIT:-7000}"
watcher="stop at the first failure"

if [ -z "${GH_TOKEN:-}" ] || [ -z "$repo" ] || [ -z "$own" ] || [ -z "$sha" ] || [ -z "$event" ] || [ -z "$branch" ]; then
	echo "::warning::stop-on-failure: a required input is missing; nothing is watched"
	exit 0
fi
case "$sha" in *[!0-9a-f]* | "") echo "::warning::stop-on-failure: the commit is not a hash; nothing is watched"; exit 0 ;; esac
case "$own" in *[!0-9]*) echo "::warning::stop-on-failure: the run id is not a number; nothing is watched"; exit 0 ;; esac
case "$event" in
pull_request)
	case "$pr" in *[!0-9]* | "") echo "::warning::stop-on-failure: a pull request without a number; nothing is watched"; exit 0 ;; esac
	;;
merge_group) pr="" ;;
*) echo "::warning::stop-on-failure: the event is neither a pull request nor a merge group; nothing is watched"; exit 0 ;;
esac
case "$branch" in refs/*) echo "::warning::stop-on-failure: the branch has refs/ in front of it; nothing is watched"; exit 0 ;; esac
for n in "$interval" "$warmup" "$limit"; do
	case "$n" in *[!0-9]* | "") echo "::warning::stop-on-failure: a time is not a number; nothing is watched"; exit 0 ;; esac
done

# The runs that are this one's, as lines `id status path`. Reads the list of runs of the commit and of the event
# and keeps those of this repository, this branch and, on a pull request, this number.
own_runs() {
	local listing
	listing="$(gh api "repos/${repo}/actions/runs?head_sha=${sha}&event=${event}&per_page=100" 2>/dev/null)" || return 1
	printf '%s' "$listing" | REPO="$repo" SHA="$sha" EV="$event" BRANCH="$branch" PR="$pr" jq -r '
		.workflow_runs[]?
		| select(.head_sha == env.SHA and .event == env.EV
			and (.head_repository.full_name // "") == env.REPO
			and (.head_branch // "") == env.BRANCH
			and (env.EV != "pull_request" or any(.pull_requests[]?; (.number | tostring) == env.PR)))
		| [(.id | tostring), (.status // "-"), (.path // "-" | split("@")[0])] | @tsv' 2>/dev/null
}

# One look: the check runs of the commit, as lines `status conclusion run-id name`. The run is in the
# address of the job (.../actions/runs/<run>/job/<job>); the ones that are not a job of Actions are dropped.
look() {
	local out
	out="$(gh api "repos/${repo}/commits/${sha}/check-runs?per_page=100&filter=latest" 2>/dev/null)" || return 1
	printf '%s' "$out" | jq -r '
		.check_runs[]?
		| select((.app.slug // "") == "github-actions")
		| ((.details_url // "") | capture("/actions/runs/(?<run>[0-9]+)/") // {run: ""}) as $r
		| select($r.run != "")
		| [(.status // "-"), (.conclusion // "-"), $r.run, (.name // "-")] | @tsv' 2>/dev/null
}

# Cancel what has not finished among this pull request's or merge group's runs that the list allows, then this run.
cut_runs() {
	local runs id st path listed workflow
	if ! runs="$(own_runs)"; then
		echo "::warning::stop-on-failure: the run list could not be read; nothing else is cancelled"
		runs=""
	fi
	while IFS=$'\t' read -r id st path; do
		[ -n "$id" ] || continue
		[ "$id" != "$own" ] || continue
		case "$st" in queued | in_progress | waiting | pending) ;; *) continue ;; esac
		listed=0
		for workflow in "${CANCELLABLE[@]}"; do
			[ "$path" = "$workflow" ] && listed=1
		done
		if [ "$listed" -eq 0 ]; then
			echo "kept: run $id of ${path:-an unnamed workflow} is not on the list"
			continue
		fi
		if gh api -X POST "repos/${repo}/actions/runs/${id}/cancel" >/dev/null 2>&1; then
			echo "cancelled: run $id of $path"
		else
			echo "::warning::stop-on-failure: run $id of $path could not be cancelled"
		fi
	done <<<"$runs"
	if gh api -X POST "repos/${repo}/actions/runs/${own}/cancel" >/dev/null 2>&1; then
		echo "cancelled: this run, $own"
	else
		echo "::warning::stop-on-failure: this run, $own, could not be cancelled"
	fi
}

# Time is counted in looks times the interval, not read from a clock: it is exact and a test can run it
# without waiting. The requests are fast against the interval, and the job has its own timeout.
looks=0
mine=""        # the runs known to be this one's, one id per line
notmine=" "    # the runs known not to be, ids between spaces: their failures are nobody's here
while :; do
	elapsed=$((looks * interval))
	looks=$((looks + 1))
	if checks="$(look)"; then
		failed=""
		while IFS=$'\t' read -r st conclusion run name; do
			[ "$st" = completed ] || continue
			[ "$conclusion" = failure ] || [ "$conclusion" = timed_out ] || continue
			case "$notmine" in *" $run "*) continue ;; esac
			if ! printf '%s\n' "$mine" | grep -qx "$run"; then
				if listed="$(own_runs)"; then
					mine="$(printf '%s\n' "$listed" | cut -f1)"
				else
					echo "::warning::stop-on-failure: the run list could not be read, so the failure of run $run is not attributed"
					continue
				fi
				if ! printf '%s\n' "$mine" | grep -qx "$run"; then
					notmine="$notmine$run "
					continue
				fi
			fi
			failed="${failed}${name} (run ${run})"$'\n'
		done <<<"$checks"
		if [ -n "$failed" ]; then
			echo "::notice::stop-on-failure: a job failed after ${elapsed} s of watching, so the rest is cut:"
			printf '%s' "$failed" | sed 's/^/  /'
			cut_runs
			exit 0
		fi
		# Nothing is left to wait for when every job but this one has finished, once the other workflows had
		# time to start: they are created a little after this one.
		active="$(printf '%s\n' "$checks" | awk -F'\t' -v w="$watcher" '$1 != "completed" && index($4, w) == 0 { n++ } END { print n + 0 }')"
		if [ "$active" -eq 0 ] && [ "$elapsed" -ge "$warmup" ]; then
			echo "stop-on-failure: every job of the commit has finished and none failed after ${elapsed} s; nothing to cut"
			exit 0
		fi
	else
		echo "::warning::stop-on-failure: the check runs of the commit could not be read; looking again"
	fi
	if [ "$elapsed" -ge "$limit" ]; then
		echo "::warning::stop-on-failure: the limit of ${limit} s of watching was reached; nothing is cut"
		exit 0
	fi
	sleep "$interval"
done
