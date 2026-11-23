#!/usr/bin/env bash
#
# Fail when a job of this run carries a warning that something it uses is
# deprecated or is going away. GitHub reports those as annotations on the job
# (a Node runtime an action still uses, a command that was retired), and a run
# that is green with such a warning is a run that stops working on a date
# nobody wrote down.
#
# Inputs (env): GH_TOKEN (read access to actions and checks), REPO, RUN_ID.
#
# It reads every job of the run, and for each the annotations of level `warning`
# or `failure`, and fails when one of them matches PATTERN. A notice is not read:
# only a warning says something has to change. A listing or an annotation read
# that fails is a failure too: a check that could not look has not found nothing.
set -uo pipefail

repo="${REPO:?REPO}"
run="${RUN_ID:?RUN_ID}"
case "$run" in *[!0-9]* | "") echo "::error::RUN_ID is not a number" >&2; exit 1 ;; esac
PATTERN='deprecat|no longer supported|will be (removed|disabled|retired)|forced to run on node'

# The filters run in jq, on what gh printed, so a test can plant the answers.
set -o pipefail
if ! jobs="$(gh api --paginate "repos/${repo}/actions/runs/${run}/jobs?per_page=100" | jq -r '.jobs[]? | [(.id | tostring), .name] | @tsv')"; then
	echo "::error::the jobs of run $run could not be listed, so their warnings were not read"
	exit 1
fi

found=0
while IFS=$'\t' read -r id name; do
	[ -n "$id" ] || continue
	if ! notes="$(gh api --paginate "repos/${repo}/check-runs/${id}/annotations?per_page=100" | jq -r '.[]? | select(.annotation_level == "warning" or .annotation_level == "failure") | .message')"; then
		echo "::error::the annotations of '$name' could not be read, so they were not checked"
		found=1
		continue
	fi
	while IFS= read -r line; do
		[ -n "$line" ] || continue
		if printf '%s\n' "$line" | grep -qiE "$PATTERN"; then
			echo "::error::'$name' carries a deprecation warning: $line"
			found=1
		fi
	done <<<"$notes"
done <<<"$jobs"

[ "$found" -eq 0 ] && echo "no job of run $run carries a deprecation warning"
exit "$found"
