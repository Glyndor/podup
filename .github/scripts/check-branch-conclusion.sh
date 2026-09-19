#!/usr/bin/env bash
#
# Decide whether the newest completed run of a workflow on a branch is a
# pass. Extracted from the step in `reusable-branch-health.yml` so the
# conclusion logic can be exercised against planted API answers without
# `gh` or the network.
#
# Reads the GitHub API response on stdin (the JSON returned by
# `repos/:owner/:repo/actions/workflows/:workflow/runs?branch=:branch&status=completed&per_page=30`),
# prints a one-line summary on stdout, and exits 0 only when the
# newest run's `conclusion` is `success`.
#
# Exits 1 otherwise. The line printed is the same one the workflow step
# surfaces, so a planted test can match on it.
#
# The page is sorted by `created_at` inside `jq` because the API's
# filtered page is not always newest-first, which gave the
# schedule-freshness gate two false reds, on 2026-09-08 and 2026-09-17. Runs
# whose conclusion is `cancelled` are passed over because on a push run
# they mean a newer push superseded this one with `cancel-in-progress`,
# and the newer run answers for the branch. Counting the cancelled one
# as a failure would flip the guard red over a run that has already
# been replaced. `skipped` stays a failure: a skipped run is the
# workflow's own gate deciding not to exercise the branch it claims to
# protect.

set -euo pipefail

workflow=${WORKFLOW:?WORKFLOW env var is required}
branch=${BRANCH:?BRANCH env var is required}

# Sort `.workflow_runs` by `created_at`, newest first, and emit one
# tab-separated `conclusion\thtml_url` row per run. The count of
# `cancelled` rows is emitted last, on its own line, so the shell can
# read both with `sed -n` and `tail -n 1`.
#
# `created_at // ""` keeps the field a comparable string even when the
# page is empty or a row is missing it; both fall to the bottom of the
# sort and are filtered out by the `select` below. `select(.conclusion)`
# drops runs that are not yet concluded (which the `status=completed`
# filter on the API is supposed to keep out anyway); counting them as
# cancelled would not match what the page is meant to contain.
sorted="$(jq -r '
  [.workflow_runs[]? | {created_at: (.created_at // ""), conclusion: (.conclusion // ""), html_url: (.html_url // "")}]
  | sort_by(.created_at) | reverse
  | (map(select(.conclusion == "cancelled")) | length) as $cancelled_count
  | (map(select(.conclusion != "cancelled")) | .[] | "\(.conclusion)\t\(.html_url)"), $cancelled_count
')"

# Last line is the count of cancelled rows; the rows above it are the
# non-cancelled runs, newest first.
cancelled_count="$(printf '%s\n' "$sorted" | tail -n 1)"
runs="$(printf '%s\n' "$sorted" | sed '$d')"

if [ -z "$runs" ]; then
	if [ "$cancelled_count" -gt 0 ]; then
		echo "::error::All ${cancelled_count} completed runs of ${workflow} on record for branch ${branch} were cancelled: no verdict."
		echo "A protected branch whose every run is cancelled is not a branch to release from." >&2
	else
		echo "::error::No completed run of ${workflow} on record for branch ${branch}."
		echo "A protected branch with no completed run is not a branch to release from." >&2
	fi
	exit 1
fi

# Take the first run that was not cancelled, split it on the tab, and name what
# passed over before the verdict line so the operator sees the count.
first="$(printf '%s\n' "$runs" | sed -n '1p')"
conclusion="$(printf '%s\n' "$first" | cut -f1)"
run_url="$(printf '%s\n' "$first" | cut -f2)"

if [ "$cancelled_count" -gt 0 ]; then
	echo "Passed over ${cancelled_count} cancelled run(s) of ${workflow} on ${branch}; newest remaining run follows."
fi

echo "Newest completed run of ${workflow} on ${branch}: conclusion=${conclusion}, url=${run_url}"

case "$conclusion" in
	success)
		echo "Within the success threshold."
		exit 0
		;;
	failure)
		echo "::error::Newest completed run of ${workflow} on ${branch} failed: ${run_url}"
		exit 1
		;;
	skipped)
		# Skipped is the workflow's own gate deciding not to run for the
		# branch. On a branch whose `on:` is supposed to cover it, that is
		# the gate not exercising the protection the comment claims.
		echo "::error::Newest completed run of ${workflow} on ${branch} was skipped: ${run_url}"
		echo "A skipped run on a protected branch is the workflow deciding not to exercise the gate." >&2
		exit 1
		;;
	*)
		# Any other conclusion (timed_out, action_required, neutral,
		# startup_failure, stale) is also not success. The workflow does
		# not enumerate them, and this script is the single place that
		# decides which ones to enumerate.
		echo "::error::Newest completed run of ${workflow} on ${branch} has unexpected conclusion ${conclusion}: ${run_url}"
		exit 1
		;;
esac
