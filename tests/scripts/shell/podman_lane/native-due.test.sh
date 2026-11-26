#!/usr/bin/env bash
#
# native-due.sh decides whether a scheduled slot of the Podman lane runs the native lane.
#
# A fake `gh` serves the list of scheduled runs and the jobs of each from a folder, so each case
# says which runs exist, when they started and what their native lane did. The rules held here:
#
#   - a green scheduled run whose native lane passed, less than the window ago, excuses the slot;
#   - this very run never excuses itself;
#   - a green run whose native lane was skipped (an excused slot) or failed does not count: a
#     chain of excused slots must not read as a lane that ran;
#   - a run older than the window does not count;
#   - whatever cannot be read (the list, a time, the jobs) leaves the lane due.
#
# Each case that must come out `false` is also run with the one rule that makes it so taken away,
# in a copy of the script, and must then come out `true`.
#
# Requires: jq (the fake `gh` runs the real filter of the script over JSON like the API's).
set -u
export LC_ALL=C

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
SCRIPT="$HERE/.github/scripts/podman_lane/native-due.sh"
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

NOW=1795780800 # 2026-11-26T12:00:00Z
mkdir -p "$WORK/bin"
cat > "$WORK/bin/gh" <<'GH'
#!/usr/bin/env bash
# gh api <path> --jq <filter>: the path says which fixture answers, and the filter of the script
# is run over it with the real jq, so a filter that selects the wrong thing is seen.
path="$2"
filter="$4"
case "$path" in
	*/workflows/podman-lane.yml/runs*)
		[ ! -e "$FIX/runs.fail" ] || exit 1
		# `status=success` in the query keeps the runs that ended green, as the API does.
		case "$path" in
			*status=success*) jq -r ".workflow_runs |= map(select(.conclusion == \"success\")) | $filter" "$FIX/runs.json" ;;
			*) jq -r "$filter" "$FIX/runs.json" ;;
		esac ;;
	*/actions/runs/*/jobs*)
		id="${path#*/actions/runs/}"
		id="${id%%/*}"
		[ ! -e "$FIX/jobs-$id.fail" ] || exit 1
		jq -r "$filter" "$FIX/jobs-$id.json" ;;
	*) exit 1 ;;
esac
GH
chmod +x "$WORK/bin/gh"

hours_ago() { date -u -d "@$((NOW - $1 * 3600))" +%Y-%m-%dT%H:%M:%SZ; }

F="$WORK/f"
runs() { # <id>:<created>[:<conclusion>] ... -> the list of runs the API would give; green unless said
	rm -rf "$F"
	mkdir -p "$F"
	local item first=1
	printf '{"workflow_runs":[' > "$F/runs.json"
	for item in "$@"; do
		[ "$first" -eq 1 ] || printf ',' >> "$F/runs.json"
		first=0
		local id="${item%%:*}" rest="${item#*:}" conclusion=success
		case "$rest" in *Z:*) conclusion="${rest##*Z:}"; rest="${rest%%Z:*}Z" ;; esac
		printf '{"id":%s,"created_at":"%s","conclusion":"%s"}' "$id" "$rest" "$conclusion" >> "$F/runs.json"
	done
	printf ']}' >> "$F/runs.json"
}
jobs() { # <run id> <conclusion of podman-native, or - for no such job>
	if [ "$2" = - ]; then
		printf '{"jobs":[{"name":"decide","conclusion":"success"},{"name":"podman-vm (6)","conclusion":"success"}]}' > "$F/jobs-$1.json"
	else
		printf '{"jobs":[{"name":"decide","conclusion":"success"},{"name":"podman-vm (6)","conclusion":"success"},{"name":"podman-native","conclusion":"%s"}]}' "$2" > "$F/jobs-$1.json"
	fi
}
decide() { # <script> -> due=...
	local out rc
	out="$(FIX="$F" PATH="$WORK/bin:$PATH" GH_REPO=o/r RUN_ID=999 NOW="$NOW" GITHUB_OUTPUT="$F/out" bash "$1" 2> /dev/null)"
	rc=$?
	[ "$rc" -eq 0 ] || { echo "exit $rc"; return; }
	tail -1 "$F/out" 2> /dev/null
}

runs "100:$(hours_ago 4)"; jobs 100 success
check "a native lane that passed 4 hours ago excuses the slot" "due=false" "$(decide "$SCRIPT")"
runs "100:$(hours_ago 4)"; jobs 100 skipped
check "a green run whose native lane was skipped does not excuse it" "due=true" "$(decide "$SCRIPT")"
runs "100:$(hours_ago 4)"; jobs 100 failure
check "a native lane that failed does not excuse it" "due=true" "$(decide "$SCRIPT")"
runs "100:$(hours_ago 4)"; jobs 100 -
check "a run with no native job does not excuse it" "due=true" "$(decide "$SCRIPT")"
runs "100:$(hours_ago 20)"; jobs 100 success
check "a pass 20 hours ago is older than the window" "due=true" "$(decide "$SCRIPT")"
runs "100:$(hours_ago 17)"; jobs 100 success
check "a pass 17 hours ago is inside the window" "due=false" "$(decide "$SCRIPT")"
runs "999:$(hours_ago 1)"; jobs 999 success
check "this run never excuses itself" "due=true" "$(decide "$SCRIPT")"
runs "101:$(hours_ago 2)" "100:$(hours_ago 5)"; jobs 101 skipped; jobs 100 success
check "an excused slot in between does not hide the run that passed" "due=false" "$(decide "$SCRIPT")"
runs
check "no scheduled run at all leaves it due" "due=true" "$(decide "$SCRIPT")"
runs "100:$(hours_ago 4)"; jobs 100 success; touch "$F/runs.fail"
check "a list that cannot be read leaves it due" "due=true" "$(decide "$SCRIPT")"
runs "100:$(hours_ago 4)"; jobs 100 success; touch "$F/jobs-100.fail"
check "jobs that cannot be read leave it due" "due=true" "$(decide "$SCRIPT")"
runs "100:not-a-time"; jobs 100 success
check "a time that cannot be read leaves it due" "due=true" "$(decide "$SCRIPT")"
runs "100:$(hours_ago 4)"; jobs 100 success
check "a window that is not a number is refused" "exit 2" "$(FIX="$F" PATH="$WORK/bin:$PATH" GH_REPO=o/r RUN_ID=999 NOW="$NOW" WINDOW_HOURS=soon GITHUB_OUTPUT="$F/out" bash "$SCRIPT" > /dev/null 2>&1; echo "exit $?")"
runs "100:$(hours_ago 4)"; jobs 100 success
check "the window is a setting: 3 hours excludes a pass 4 hours ago" "due=true" "$(FIX="$F" PATH="$WORK/bin:$PATH" GH_REPO=o/r RUN_ID=999 NOW="$NOW" WINDOW_HOURS=3 GITHUB_OUTPUT="$F/out" bash "$SCRIPT" > /dev/null 2>&1; tail -1 "$F/out")"

# The rules, one at a time: the case that needs the rule is run on a copy of the script without it,
# and must read the other way. A plant that changes nothing is a failure of the plant.
plant() { # <description> <sed expression> <expected on the plant>   (the case is the one set up before the call)
	local copy="$WORK/plant.sh" real
	sed -E "$2" "$SCRIPT" > "$copy"
	if cmp -s "$copy" "$SCRIPT"; then
		check "$1: the plant changes the script" changed same
		return
	fi
	real="$(decide "$SCRIPT")"
	check "$1: the real script" "$4" "$real"
	check "$1: without the rule" "$3" "$(decide "$copy")"
}
runs "100:$(hours_ago 30)"; jobs 100 success
plant "the window not read" 's/\[ "\$when" -ge "\$limit" \] \|\| continue/true/' due=false due=true
runs "100:$(hours_ago 4)"; jobs 100 skipped
plant "the result of the native job not read" 's/if \[ "\$result" = success \]/if [ -n "$id" ]/' due=false due=true
runs "100:$(hours_ago 4)"; jobs 100 -
plant "the native job not selected by name" 's/select\(\.name == "podman-native"\)/select(.name != "x")/' due=false due=true
runs "999:$(hours_ago 1)"; jobs 999 success
plant "this run not skipped" 's/\[ "\$id" != "\$RUN_ID" \] \|\| continue/true/' due=false due=true
runs "100:$(hours_ago 4):failure"; jobs 100 success
plant "only green runs not asked for" 's/&status=success//' due=false due=true

echo
echo "passed: $pass  failed: $fail"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
