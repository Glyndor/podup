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
#      page is a buffer to sort). On `develop` the runs of the merge
#      queue for that branch are read too, since the queue ran the
#      suite on the very commit that landed and a push would repeat it.
#   3. Pick the run to judge, once, without waiting:
#        - the newest finished run whose head_sha is the head: that is the
#          verdict for the head;
#        - when the head has a run that is still queued or running, the
#          newest FINISHED run before it that concluded in a pass or a
#          failure of any kind (`success`, `failure`, `timed_out`,
#          `startup_failure`), as long as it is no older than
#          FALLBACK_HOURS (48) and its commit is an ANCESTOR of the head
#          (the compare API says `ahead` or `identical`): a run for a
#          commit that is not in the history of the head says nothing about
#          it. A compare call that FAILS is unknown, and unknown is not
#          green: the script stops there and exits 1, and does not try an
#          older run in its place. A run of the head that has been pending for more than
#          PENDING_HOURS (24) is a suite that is stuck, and that is red;
#        - when the head has NO run at all, the commit is given GRACE_HOURS
#          (3) for its suite to be created: younger than that the answer is
#          a warning and a pass, older is red ("head without CI"). Without
#          this a bad commit that never ran anything would inherit the
#          success of the one before it.
#        - a cancelled or skipped run of another commit says nothing about
#          the branch and is passed over; one of the head's own is red.
#      With runners saturated a suite takes longer than any wait this job
#      could afford, so the job does not wait: it judges what has finished.
#   4. Decide on the conclusion. `success` exits 0. `failure` looks at
#      the run's own jobs and exits 0 when every failed job is a
#      branch-health one; otherwise it exits 1 and names the failing
#      jobs. `timed_out`, `startup_failure`, `cancelled`, `skipped` and
#      anything else exit 1 with a message that names the run. Red means
#      a finished run failed, or a head has no CI at all; never that a
#      suite is merely still running.
#
# Time is read from NOW_EPOCH when set (a test plants it), else from `date`.

set -euo pipefail

workflow=${WORKFLOW:?WORKFLOW env var is required}
branch=${BRANCH:?BRANCH env var is required}
repo=${REPO:?REPO env var is required}

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

# 2-3. Read the runs and pick the one to judge. The URL's `event=push` keeps the
# page from filling with runs the filter would discard (a `pull_request` run on a
# PR whose source branch is the watched branch carries the same `head_sha` as the
# push run on that head); the jq select repeats the filter against the response
# bytes as defense in depth, so a stubbed API that ignores the query is still
# asked the same question. Sorting by (created_at, id) puts the newest first, so
# an older green run for a different commit is NOT the verdict for a head that
# has a finished run of its own.
url="repos/${repo}/actions/workflows/${workflow}/runs?branch=${branch}&event=push&per_page=30"
runs_filter='[.workflow_runs[]? | select(.event=="push")]'
queue_url="repos/${repo}/actions/workflows/${workflow}/runs?event=merge_group&per_page=30"
queue_filter='[.workflow_runs[]? | select(.event=="merge_group" and ((.head_branch // "") | startswith("gh-readonly-queue/'"${branch}"'/")))]'

# A failed `gh api` is "no answer": it must not abort the script, and it must not
# turn into red either, because nothing was learnt about the branch. The warning
# names it so an operator reading the log can tell why a verdict is missing.
read_runs() { # <variable> <url> <filter>
	local out
	if out="$(gh api "$2" --jq "$3")" && [ -n "$out" ]; then
		printf -v "$1" '%s' "$out"
	else
		echo "::warning::gh api call failed or answered nothing (${2}); treating it as no runs."
		printf -v "$1" '[]'
	fi
}
pushed='[]'
queued='[]'
read_runs pushed "$url" "$runs_filter"
# `develop` takes no push run of this workflow: the merge queue ran it on the
# very commit that landed, and a push would repeat that. The runs of the queue
# for that branch are its history.
if [ "$branch" = develop ]; then
	read_runs queued "$queue_url" "$queue_filter"
fi

now="${NOW_EPOCH:-$(date +%s)}"
grace_hours="${GRACE_HOURS:-3}"
pending_hours="${PENDING_HOURS:-24}"
fallback_hours="${FALLBACK_HOURS:-48}"
all="$(printf '%s\n%s\n' "$pushed" "$queued" | jq -cs 'add | sort_by(.created_at, .id) | reverse' 2>/dev/null)" || all="[]"
[ -n "$all" ] || all="[]"

# A run as one line: id, status, conclusion, number, head_sha, created_at, url. A tab is white space
# to `read`, so an empty field would be swallowed and shift the rest: none is empty.
# shellcheck disable=SC2016 # a jq program, not shell
fields='[(.id | tostring), (.status // "-"), (.conclusion // "-"), (.run_number | tostring), (.head_sha // "-"), (.created_at // "-"), (.html_url // "-")] | @tsv'
# shellcheck disable=SC2016 # a jq program, not shell
mine_done="$(jq -r --arg head "$head_sha" "[.[] | select(.head_sha == \$head and .status == \"completed\")][0] // empty | $fields" <<<"$all")"
# shellcheck disable=SC2016 # a jq program, not shell
mine_pending="$(jq -r --arg head "$head_sha" "[.[] | select(.head_sha == \$head and .status != \"completed\")][0] // empty | $fields" <<<"$all")"

epoch_of() { date -d "$1" +%s 2>/dev/null || echo 0; }
run=""
origin=""
if [ -n "$mine_done" ]; then
	run="$mine_done"
	origin="head"
elif [ -n "$mine_pending" ]; then
	# The head's suite is queued or running: judge the last run that finished before it, but
	# never let a suite that is stuck go unseen.
	IFS=$'\t' read -r _ _ _ pnumber _ pcreated purl <<<"$mine_pending"
	age=$((now - $(epoch_of "$pcreated")))
	if [ "$age" -gt $((pending_hours * 3600)) ]; then
		echo "::error::${workflow} on ${branch} for commit ${short}: run #${pnumber} has been queued or running for more than ${pending_hours} hours (started ${pcreated})."
		echo "  ${purl}" >&2
		echo "  A suite that long is stuck, and nothing finished can answer for this commit." >&2
		exit 1
	fi
	cutoff=$((now - fallback_hours * 3600))
	# shellcheck disable=SC2016 # a jq program, not shell
	candidates="$(jq -r --arg head "$head_sha" --argjson cutoff "$cutoff" "[.[] | select(.head_sha != \$head and .status == \"completed\"
		and (.conclusion == \"success\" or .conclusion == \"failure\" or .conclusion == \"timed_out\" or .conclusion == \"startup_failure\")
		and ((.created_at | fromdateiso8601) >= \$cutoff))][:10][] | $fields" <<<"$all")"
	while IFS= read -r line; do
		[ -n "$line" ] || continue
		IFS=$'\t' read -r _ _ _ _ csha _ _ <<<"$line"
		# A run of a commit that is not in the history of the head says nothing about it.
		# A call that FAILS is not "not an ancestor": the newest candidate is unknown, and moving on to an
		# older one could answer green for a commit whose newer run was red. Unknown is not green.
		if ! rel="$(gh api "repos/${repo}/compare/${csha}...${head_sha}" --jq '.status' 2>/dev/null)" || [ -z "$rel" ]; then
			echo "::warning::gh api call failed or answered nothing (repos/${repo}/compare/${csha}...${head_sha}); an older run is not tried in its place."
			echo "::error::${workflow} on ${branch} for commit ${short}: the head's run has not finished, and it could not be told whether the newest finished run (${csha:0:7}) is an ancestor of it, so there is no verdict."
			echo "  Unknown is not green: the next tick asks again." >&2
			exit 1
		fi
		case "$rel" in
			ahead | identical)
				run="$line"
				origin=earlier
				break
				;;
		esac
	done <<<"$candidates"
	if [ -z "$run" ]; then
		echo "::warning::the ${workflow} run for ${short} on ${branch} has not finished, and no run of an ancestor of it finished in the last ${fallback_hours} hours, so there is no verdict yet"
		exit 0
	fi
else
	# The head has no run at all. A commit that was just pushed has not had time to start one;
	# one that is older than the grace never ran anything, and the success of the commit before
	# it is not its own.
	cdate="$(gh api "repos/${repo}/commits/${head_sha}" --jq '.commit.committer.date // ""' 2>/dev/null)" || cdate=""
	if [ -z "$cdate" ]; then
		echo "::warning::could not read the date of commit ${short}, and ${workflow} has no run for it on ${branch}; there is no verdict"
		exit 0
	fi
	age=$((now - $(epoch_of "$cdate")))
	if [ "$age" -gt $((grace_hours * 3600)) ]; then
		echo "::error::head without CI: ${workflow} has no run for commit ${short} on ${branch}, and the commit is older than ${grace_hours} hours (committed ${cdate})."
		echo "  Nothing tested this commit, so the runs of the commits before it do not answer for it." >&2
		echo "  Open the workflow run list on ${branch}: the suite never started, or its run was deleted." >&2
		exit 1
	fi
	echo "::warning::${workflow} has no run for commit ${short} on ${branch} yet (committed ${cdate}, inside the ${grace_hours}-hour grace); there is no verdict"
	exit 0
fi
run="${run}"$'\t'"${origin}"

IFS=$'\t' read -r id _ conclusion number rsha created rurl origin <<<"$run"
rshort="${rsha:0:7}"
if [ "$origin" = earlier ]; then
	# The head has no finished run: say what is judged instead, so the log of a
	# green job does not read as if the head itself had passed.
	echo "::notice::the ${workflow} run for ${short} on ${branch} has not finished; judging the last one that did: run #${number} for commit ${rshort}."
fi

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
		# The run is red, but possibly only because of its own
		# branch-health job. The gate on `main` reads `develop`'s push
		# run and the gate on `develop` reads `main`'s, so a run that
		# is red only on its branch-health job is red because the
		# other branch was red when it ran. Counting that would let
		# each branch hold the other red indefinitely, since a re-run
		# of either reads the other's red. Decide on the run's own
		# work instead: fetch its jobs, drop the ones whose name
		# starts with `branch health`, and exit green if nothing is
		# left, red with the failing job names if it is. The fetch
		# tolerates a transient API failure the same way the polling
		# loop above does: a 5xx or a rate limit must not abort the
		# script here any more than it does there.
		jobs_url="repos/${repo}/actions/runs/${id}/jobs?per_page=100"
		jobs_filter='[.jobs[]?
			| select((.conclusion // "") != "success"
				and (.conclusion // "") != "skipped")]
			| map(.name) | .[]'
		jobs=""
		if jobs="$(gh api "$jobs_url" --jq "$jobs_filter")"; then
			:
		else
			gh_rc=$?
			echo "::warning::gh api call for jobs failed (exit ${gh_rc}); treating as no answer."
			jobs=""
		fi
		if [ -z "$jobs" ]; then
			# The run is red but no failed jobs were visible. Either
			# the jobs call failed or the run's own jobs list was
			# empty, and either way the verdict on whether
			# branch-health was the only failure is unknown. An
			# unknown verdict is not green.
			echo "::error::${workflow} on ${branch} for commit ${rshort}: run #${number} concluded ${conclusion} (started ${created})."
			echo "  ${rurl}" >&2
			echo "  Could not see the run's failed jobs to rule out the branch-health gate," >&2
			echo "  so the verdict is unknown rather than green." >&2
			exit 1
		fi
		non_bh="$(printf '%s\n' "$jobs" | grep -v '^branch health' || true)"
		if [ -z "$non_bh" ]; then
			# Only branch-health jobs failed: the latch, not the
			# branch. One line, naming the branch-health jobs so an
			# operator can tell at a glance that the run is red only
			# because of them. Exit 0 so the gate that called this
			# script reads the verdict as the branch's own work
			# having passed, which is what it did.
			bh_failed="$(printf '%s\n' "$jobs" | awk 'NR > 1 { printf ", " } { printf "%s", $0 }')"
			echo "${workflow} on ${branch} for commit ${rshort}: run #${number} concluded ${conclusion} only on branch-health job(s) (${bh_failed}); the branch's own code passed."
			exit 0
		fi
		# `paste -sd ', '` would alternate the two characters as
		# separators ("a,b c"), so join with awk.
		failed="$(printf '%s\n' "$non_bh" | awk 'NR > 1 { printf ", " } { printf "%s", $0 }')"
		echo "::error::${workflow} on ${branch} for commit ${rshort}: run #${number} concluded ${conclusion} (started ${created})."
		echo "  ${rurl}" >&2
		echo "  Failed jobs: ${failed}" >&2
		exit 1
		;;
	timed_out | startup_failure)
		# A suite that timed out or never started did not pass. These are failures of the run
		# as a whole, with no failed job to rule the latch out, so there is nothing to excuse.
		echo "::error::${workflow} on ${branch} for commit ${rshort}: run #${number} concluded ${conclusion} (started ${created})."
		echo "  ${rurl}" >&2
		echo "  A run that ${conclusion//_/ } is a failure: the suite did not reach a pass." >&2
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
