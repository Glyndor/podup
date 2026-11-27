#!/usr/bin/env bash
#
# Decide whether the native lane has to run on this scheduled slot.
#
# The Podman lane has three cron slots a day. The first one also boots the VM; the other
# two exist only so that a cron GitHub does not fire does not leave a day with no look at
# the Podman that apt.glyndor.net serves. A slot runs the native lane unless a scheduled run
# of this workflow, other than this one, finished green with the native lane green inside it
# less than WINDOW_HOURS ago, so a healthy day costs one native run and a day with a dead
# first slot costs one as well, from the next slot.
#
#   native-due.sh      writes `due=true` or `due=false` to $GITHUB_OUTPUT
#
# Environment:
#   GH_REPO        owner/name (required)
#   RUN_ID         id of this run, which never counts (required)
#   GH_TOKEN       for gh
#   WINDOW_HOURS   how recent a green native run excuses this one; 18 by default. It is under
#                  a day so that the first slot of tomorrow always runs after the first slot
#                  of today, and over the gap between two slots so that the slots of the same
#                  day excuse each other.
#   NOW            epoch seconds, for tests
#
# Whatever cannot be read says `due=true`: a lane that runs twice costs minutes, a lane that is
# skipped on a guess is the silence this exists to prevent.
set -euo pipefail
export LC_ALL=C

: "${GH_REPO:?GH_REPO is required}"
: "${RUN_ID:?RUN_ID is required}"
window="${WINDOW_HOURS:-18}"
now="${NOW:-$(date +%s)}"
out="${GITHUB_OUTPUT:-/dev/stdout}"
[[ "$window" =~ ^[0-9]+$ ]] || { echo "::error::WINDOW_HOURS is not a number: $window" >&2; exit 2; }

say() { printf 'due=%s\n' "$1" >> "$out"; }
due() { echo "native lane due: $1"; say true; exit 0; }

runs="$(gh api "repos/${GH_REPO}/actions/workflows/podman-lane.yml/runs?event=schedule&status=success&per_page=30" \
	--jq '.workflow_runs[] | [.id, .created_at] | @tsv')" || due "the list of runs could not be read"

limit=$((now - window * 3600))
while IFS=$'\t' read -r id created; do
	[ -n "$id" ] || continue
	[ "$id" != "$RUN_ID" ] || continue
	when="$(date -u -d "$created" +%s 2> /dev/null)" || due "the time of run $id could not be read ($created)"
	[ "$when" -ge "$limit" ] || continue
	# A run that is green is not a run whose native lane was: the lane of a slot that was excused is skipped.
	result="$(gh api "repos/${GH_REPO}/actions/runs/${id}/jobs?per_page=100" \
		--jq '[.jobs[] | select(.name == "podman-native") | .conclusion] | first // ""')" || due "the jobs of run $id could not be read"
	if [ "$result" = success ]; then
		echo "native lane not due: run $id ($created) passed it less than ${window} hours ago"
		say false
		exit 0
	fi
done <<< "$runs"
due "no scheduled run passed it in the last ${window} hours"
