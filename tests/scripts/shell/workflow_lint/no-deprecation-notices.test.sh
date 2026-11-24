#!/usr/bin/env bash
#
# The check that fails a run which carries a deprecation warning.
#
# A run that is green with a warning that an action's runtime is deprecated is a
# run that stops working on a date nobody wrote down. The script reads every job of
# the run and the annotations of each, and fails on a warning (or failure)
# annotation that says something is deprecated or going away. Each case plants one
# answer from the API with a fake `gh` and expects a verdict; the pagination, the
# levels and a failed read are the ways it could miss one.
#
# Requires: jq.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
script="$HERE/.github/scripts/ci/no-deprecation-notices.sh"
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

mkdir "$WORK/bin" "$WORK/api"
# The fake serves the file named after the last path segment, and fails when a
# file named FAIL-<segment> exists.
cat >"$WORK/bin/gh" <<'GH'
#!/usr/bin/env bash
url=""
for a in "$@"; do case "$a" in repos/*) url="$a" ;; esac; done
case "$url" in
	*/jobs*) seg=jobs ;;
	*/check-runs/*/annotations*) seg="annotations-$(echo "$url" | sed -E 's#.*/check-runs/([0-9]+)/.*#\1#')" ;;
	*) exit 2 ;;
esac
[ ! -e "$API/FAIL-$seg" ] || exit 1
cat "$API/$seg.json"
GH
chmod +x "$WORK/bin/gh"
reset() { rm -f "$WORK"/api/*; }
jobs() { printf '{"jobs":[%s]}' "$1" >"$WORK/api/jobs.json"; }
notes() { printf '[%s]' "$2" >"$WORK/api/annotations-$1.json"; }
note() { printf '{"annotation_level":"%s","message":"%s"}' "$1" "$2"; }
go() { env PATH="$WORK/bin:$PATH" API="$WORK/api" REPO=Glyndor/podup RUN_ID="${RUN_ID:-1}" bash "$script" >"$WORK/out.log" 2>&1; echo $?; }

reset
jobs '{"id":11,"name":"rust / Test"},{"id":12,"name":"lint"}'
notes 11 ""
notes 12 "$(note notice "Cache saved")"
check "a run with no warning passes" 0 "$(go)"

notes 11 "$(note warning "Node.js 20 actions are deprecated. Actions will be forced to run on Node.js 24.")"
check "a Node.js deprecation warning fails the run" 1 "$(go)"
check "the message names the job" 1 "$(grep -c "'rust / Test' carries a deprecation warning" "$WORK/out.log")"

notes 11 "$(note failure "The set-output command is deprecated and will be disabled soon.")"
check "a deprecation reported as a failure annotation fails too" 1 "$(go)"
notes 11 "$(note warning "This workflow will be removed on 2027-01-01")"
check "a warning that something will be removed fails" 1 "$(go)"
notes 11 "$(note warning "Node.js 20 is no longer supported")"
check "a warning that something is no longer supported fails" 1 "$(go)"
notes 11 "$(note warning "Process completed with exit code 1.")"
check "a warning about something else does not fail it" 0 "$(go)"
notes 11 "$(note notice "Node.js 20 actions are deprecated")"
check "a notice is not a warning and is not read" 0 "$(go)"

# The second job carries it.
notes 11 ""
notes 12 "$(note warning "the thing is Deprecated")"
check "a deprecation on any job fails the run, whatever its case" 1 "$(go)"

# Reads that fail.
notes 12 ""
touch "$WORK/api/FAIL-jobs"
check "a job listing that fails fails the check" 1 "$(go)"
check "...and says it could not read" 1 "$(grep -c 'could not be listed' "$WORK/out.log")"
rm -f "$WORK/api/FAIL-jobs"
touch "$WORK/api/FAIL-annotations-12"
check "an annotation read that fails fails the check" 1 "$(go)"
rm -f "$WORK/api/FAIL-annotations-12"
check "everything readable passes again" 0 "$(go)"

# Several pages: two JSON documents one after the other, as --paginate prints them.
printf '[%s]\n[%s]' "$(note warning "page one is fine")" "$(note warning "Actions are deprecated on page two")" >"$WORK/api/annotations-12.json"
check "a deprecation on the second page is found" 1 "$(go)"

# A job list that is empty or not shaped as expected is not a pass for a job it never read.
jobs ''
check "a run with no job passes (nothing to read)" 0 "$(go)"
check "a run id that is not a number fails" 1 "$(RUN_ID='1;2' go)"

echo
echo "passed: $pass  failed: $fail"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
