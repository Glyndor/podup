#!/usr/bin/env bash
#
# Decide whether the run for a branch's CURRENT head was a pass.
#
# Extracted from the step in `reusable-branch-health.yml` so the
# conclusion logic can be exercised against planted API answers without
# `gh` or the network. The reusable sets REPO, WORKFLOW, BRANCH, and
# HEAD_SHA (when known) in the environment and calls this script; the
# script does all the API work itself.
#
# Inputs (env):
#   WORKFLOW  workflow file to check, e.g. "ci.yml".
#   BRANCH    branch whose head's run is the verdict, e.g. "main".
#   REPO      "owner/repo" whose run history is read.
#   HEAD_SHA  commit to look up. Required on push (the reusable passes
#             GITHUB_SHA). On schedule or pull_request the script
#             fetches the branch head from the API; leaving HEAD_SHA
#             empty selects that path.
#   GH_TOKEN  token for `gh api`. The reusable passes GITHUB_TOKEN.
#
# Behaviour, in order:
#
#   1. Resolve the head SHA. If HEAD_SHA is empty, fetch
#      repos/${REPO}/branches/${BRANCH} and take .commit.sha.
#   2. Read repos/${REPO}/actions/workflows/${WORKFLOW}/runs
#      ?branch=${BRANCH}&event=push&per_page=30 (the API does not
#      always put the newest run at the head of a filtered page, so the
#      page is a buffer to sort).
#   3. Pick the newest run whose head_sha matches AND whose event is
#      `push`. Sort by created_at then id, so two runs sharing a second
#      still order by id.
#   4. If the run is not yet completed, wait 15 seconds and try again,
#      up to 32 attempts (eight minutes, inside the job's ten-minute
#      bound). The worst case is the suite's own runtime.
#   5. Decide on the conclusion. `success` exits 0; `failure`,
#      `cancelled`, `skipped`, and anything else exits 1, each with a
#      message that names the run.
#
# Why a `cancelled` run for the head is RED here (and was passed over
# before): the old logic took the newest completed run on the branch,
# which could answer for a commit nobody tested. The new logic takes
# the run for the head, and nothing newer can supersede a cancelled
# run for the head, so the only honest verdict is the one the run
# reached. Measured 2026-09-19: let A pass, push B, cancel B's run by
# hand with nothing replacing it, and the old script reported A's
# success as the branch's health.
#
# Why the runs call also filters by `event=push`: a `pull_request` run
# on a PR whose source branch is ${BRANCH} carries `head_sha=${branch}
# head` and shows up under `?branch=${BRANCH}` next to the `push` run
# for the same commit. The two are sorted by `created_at`, so whichever
# the API handed back newest wins. A release pull request from
# `develop` into `main` is a real case: its `pull_request` run is newer
# than the `push` run on `develop` for the same commit, and the gate
# used to read that `pull_request` run for `branch health (develop)`.
# That run is the release PR itself, and on a release that fails any
# gate inside the suite, its `ci.yml` concluded `failure` inside that
# very run. The gate then read its own run, made that run red, and
# every later reader saw a red the gate created (measured on the 5.9.5
# release push for commit 83315af). The fix is `event=push` in the
# query and `.event == "push"` in the jq filter: the verdict is the
# run the branch triggered, which is the `push` run. A `pull_request`
# run that merely carries the branch as its head is a different
# question and must not shadow it.
#
# Why the script owns the polling: the previous design asked the API
# for completed runs on the push path and the API filtered out a run
# that was still in flight for the head. The next push then read its
# own run and saw the previous, red one. Same shape happened between
# the two health callers when a real failure on one branch tripped the
# other. The fix is the same shape the channels shipped for their
# suite-on-main check on 2026-09-19.

set -euo pipefail

workflow=${WORKFLOW:?WORKFLOW env var is required}
branch=${BRANCH:?BRANCH env var is required}
repo=${REPO:?REPO env var is required}

# 32 attempts of 15 seconds is eight minutes; the job's ten-minute
# timeout covers the worst case with two minutes of headroom. Both
# bounds are tunable through the environment so a test can shrink
# them, but the production shape is fixed by the channels' check.
max_attempts=${MAX_ATTEMPTS:-32}
sleep_seconds=${SLEEP_SECONDS:-15}

# 1. Resolve the head SHA. On push the reusable passes GITHUB_SHA so
#    this call is skipped; on schedule and pull_request HEAD_SHA is
#    empty and the script reads the branch.
head_sha="${HEAD_SHA:-}"
if [ -z "$head_sha" ]; then
	head_sha="$(gh api "repos/${repo}/branches/${branch}" --jq '.commit.sha // ""')"
fi

if [ -z "$head_sha" ]; then
	echo "::error::could not determine the head SHA for branch ${branch}"
	echo "  The branch lookup returned no commit SHA. The branch may have" >&2
	echo "  been deleted, renamed, or not yet pushed to this repository." >&2
	exit 1
fi

short="${head_sha:0:7}"

# 2-3. Pick the newest push run whose head_sha matches the branch
# head. The URL's `event=push` keeps the page from filling with runs
# the filter would discard (a `pull_request` run on a PR whose source
# branch is the watched branch carries the same `head_sha` as the push
# run on that head); the jq select repeats the filter against the
# response bytes as defense in depth, so a stubbed API that ignores the
# query is still asked the same question. The filter sorts by
# (created_at, id) and takes the first entry, so an older green push
# run for a different commit is NOT the verdict. A match that is not
# yet completed returns `null` from jq's @tsv and `run` is empty, which
# is the signal the loop waits on.
url="repos/${repo}/actions/workflows/${workflow}/runs?branch=${branch}&event=push&per_page=30"
filter='[.workflow_runs[]? | select(.head_sha=="'"$head_sha"'" and .event=="push")]
	| sort_by(.created_at, .id) | reverse | .[0]
	| [(.status // ""), (.conclusion // ""), (.run_number | tostring),
	   (.head_sha // ""), (.created_at // ""), (.html_url // "")]
	| @tsv'

run=""
for attempt in $(seq 1 "$max_attempts"); do
	# A failed `gh api` is treated as "no answer this attempt" and the
	# loop keeps polling. A rate limit, a 5xx, or any transient API
	# failure must not abort the script mid-poll: a retry loop that
	# cannot survive a transient error is a retry loop that runs
	# exactly once. The `if` consumes the non-zero exit under
	# `set -euo pipefail`, so the loop's own `set -e` does not abort
	# here, and the warning makes the failure visible in the job log
	# so an operator reading the abort (if the polling bound is later
	# exhausted) can tell why.
	if run="$(gh api "$url" --jq "$filter")"; then
		:
	else
		gh_rc=$?
		echo "::warning::gh api call failed on attempt ${attempt} (exit ${gh_rc}); treating as no answer this attempt and polling on."
		run=""
	fi
	if [ -n "$run" ]; then
		IFS=$'\t' read -r status _ <<<"$run"
		if [ "$status" = "completed" ]; then
			break
		fi
		run=""
	fi
	if [ "$attempt" -lt "$max_attempts" ]; then
		sleep "$sleep_seconds"
	fi
done

# 4. No run for the head finished in time. The polling bounds are
# tuned to absorb the suite's own runtime on a push, so seeing this
# means the suite never reached a verdict for the head (still running,
# was cancelled by hand, or never started). One line, naming the
# commit so the right run is found rather than the newest of any
# status.
if [ -z "$run" ]; then
	echo "::error::no ${workflow} run for commit ${short} on ${branch} finished within ${max_attempts} attempts (each ${sleep_seconds}s)"
	echo "  This job polled the API from the instant the suite for ${head_sha}" >&2
	echo "  could have started and saw no completed verdict for that commit in" >&2
	echo "  time, so the state of ${branch} for that commit is unknown rather" >&2
	echo "  than green. Open the workflow run list on ${branch} to see whether" >&2
	echo "  a run for ${short} is still in flight or never started." >&2
	exit 1
fi

IFS=$'\t' read -r status conclusion number rsha created rurl <<<"$run"
rshort="${rsha:0:7}"

# 5. Verdict. The shape of the message matches the case the rest of
# this organisation already prints: workflow, branch, commit, run
# number, conclusion, started-at, and the URL on its own line so an
# operator can click through.
case "$conclusion" in
	success)
		echo "${workflow} on ${branch} for commit ${rshort}: run #${number} concluded ${conclusion} (started ${created})."
		exit 0
		;;
	failure)
		echo "::error::${workflow} on ${branch} for commit ${rshort}: run #${number} concluded ${conclusion} (started ${created})."
		echo "  ${rurl}" >&2
		echo "  Read that run before anything else lands on top of it: a second" >&2
		echo "  push onto a red commit buries which change was responsible." >&2
		exit 1
		;;
	cancelled)
		# A cancelled run for the branch's current head is RED. Nothing
		# newer can answer for the commit the push (or the schedule)
		# brought in, so the only honest verdict is the one the run
		# reached. The pass-over that used to swallow this case is gone
		# with the newest-run logic it belonged to.
		echo "::error::${workflow} on ${branch} for commit ${rshort}: run #${number} was cancelled (started ${created})."
		echo "  ${rurl}" >&2
		echo "  A cancelled run is the absence of a verdict, and nothing newer" >&2
		echo "  can answer for this commit. Re-run the workflow or push a fix" >&2
		echo "  and the next tick will read the new run." >&2
		exit 1
		;;
	skipped)
		# Skipped is the workflow's own gate deciding not to exercise the
		# branch it claims to protect. On a branch whose `on:` is supposed
		# to cover it, that is the gate not exercising the protection.
		echo "::error::${workflow} on ${branch} for commit ${rshort}: run #${number} was skipped (started ${created})."
		echo "  ${rurl}" >&2
		echo "  A skipped run is the workflow deciding not to exercise the gate." >&2
		exit 1
		;;
	*)
		# Any other conclusion (timed_out, action_required, neutral,
		# startup_failure, stale) is also not success. The workflow does
		# not enumerate them, and this script is the single place that
		# decides which ones to enumerate.
		echo "::error::${workflow} on ${branch} for commit ${rshort}: run #${number} has unexpected conclusion ${conclusion} (started ${created})."
		echo "  ${rurl}" >&2
		exit 1
		;;
esac
