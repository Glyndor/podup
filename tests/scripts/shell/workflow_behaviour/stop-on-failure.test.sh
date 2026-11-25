#!/usr/bin/env bash
#
# The job that cuts the runs of a commit at its FIRST failed job.
#
# It holds `actions: write`, so every case is a way it could cancel too much, cancel too late, spend
# the token, fail the job, or run code the pull request wrote. The stand-in `gh` answers the look
# (the check runs of the commit) from a numbered file per look, lists the runs from another, and
# records every cancellation.
#
#   - the first look with a failed job cuts: the unfinished runs of the commit, of the event and of this
#     repository whose workflow is on the list, then its own run; `failure` and `timed_out` are failures,
#     `cancelled`, `skipped`, `neutral` and a job allowed to fail (it concludes `success`) are not,
#     and a check that is not a job of Actions is no failure;
#   - it never cancels a workflow that is not on the list (a security analysis, a release, DCO), a run of
#     another event, another commit or another repository, or a run that has finished;
#   - a look is ONE request, so a run that is quiet costs 120 requests an hour and not more;
#   - with nothing left to wait for it ends, but only after the other workflows had time to start;
#   - a look that fails, a cancellation that is refused and a missing input never fail the job;
#   - what holds it: no third-party action, the permissions it needs and no more, the guard on the
#     event, the base commit's copy of the script on a pull request, no `needs` on it, and a list that is
#     the one of cancel-others.sh and holds no analysis and no release.
#
# Each plant breaks one of those in a copy and must turn a case red.
#
# Requires: python3 with PyYAML, jq.
# shellcheck disable=SC2016,SC2030,SC2031 # the plants hold literal text, and the sabotages run in subshells
set -u
export LC_ALL=C

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
script="$HERE/.github/scripts/ci/stop-on-failure.sh"
reusable="$HERE/.github/workflows/reusable-stop-on-failure.yml"
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

SHA=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
OTHER_SHA=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
WF=.github/workflows
mkdir "$WORK/bin"
printf '#!/usr/bin/env bash\nexit 0\n' >"$WORK/bin/sleep"
chmod +x "$WORK/bin/sleep"
cat >"$WORK/bin/gh" <<'GH'
#!/usr/bin/env bash
# A stand-in for gh. The look is the Nth of $LOOKS (look-N.json, the last one repeats); the runs come
# from $RUNS_JSON; a POST is recorded and refused when $CANCEL_FAIL is set.
[ "$1" = api ] || exit 2
if [ "$2" = -X ] && [ "$3" = POST ]; then
	[ "${CANCEL_FAIL:-0}" = 0 ] || exit 1
	echo "$4" >>"$CANCELLED"
	exit 0
fi
case "$2" in
*"/check-runs"*)
	echo "$2" >>"$LOOKED"
	[ "${LOOK_FAIL:-0}" = 0 ] || exit 1
	n=$(wc -l <"$LOOKED")
	f="$LOOKS/look-$n.json"
	[ -f "$f" ] || f="$(ls "$LOOKS"/look-*.json | sort -V | tail -1)"
	cat "$f"
	;;
*"/actions/runs?"*)
	echo "$2" >>"$LISTED"
	[ "${LIST_FAIL:-0}" = 0 ] || exit 1
	cat "$RUNS_JSON"
	;;
*) exit 2 ;;
esac
GH
chmod +x "$WORK/bin/gh"

# check <status> <conclusion|null> <run> <name> [app]: one check run.
chk() {
	local c="\"$2\""
	[ "$2" = null ] && c=null
	printf '{"status":"%s","conclusion":%s,"details_url":"https://github.com/Glyndor/podup/actions/runs/%s/job/9","name":"%s","app":{"slug":"%s"}}' "$1" "$c" "$3" "$4" "${5:-github-actions}"
}
look_file() { # <n> <check>...
	local n="$1" out='{"check_runs":[' first=1 c
	shift
	for c in "$@"; do
		[ "$first" = 1 ] || out+=','
		out+="$c"
		first=0
	done
	printf '%s]}' "$out" >"$WORK/looks/look-$n.json"
}
run_json() { # <id> <event> <status> <path> [sha] [repo] [branch] [pull request number]
	printf '{"id":%s,"event":"%s","status":"%s","path":"%s","head_sha":"%s","head_repository":{"full_name":"%s"},"head_branch":"%s","pull_requests":[{"number":%s}]}' \
		"$1" "$2" "$3" "$4" "${5:-$SHA}" "${6:-Glyndor/podup}" "${7:-feature}" "${8:-7}"
}
listing() {
	local out='{"workflow_runs":[' first=1 r
	for r in "$@"; do
		[ "$first" = 1 ] || out+=','
		out+="$r"
		first=0
	done
	printf '%s]}' "$out" >"$WORK/runs.json"
}
ids() { [ -f "$WORK/cancelled.log" ] && sed -E 's#.*/runs/([0-9]+)/cancel#\1#' "$WORK/cancelled.log" | paste -sd, - || echo none; }
go() { # [env overrides]: runs the script of $S, prints the exit status
	rm -f "$WORK/cancelled.log" "$WORK/listed.log" "$WORK/looked.log"
	: >"$WORK/looked.log"
	env PATH="$WORK/bin:$PATH" LOOKS="$WORK/looks" RUNS_JSON="$WORK/runs.json" CANCELLED="$WORK/cancelled.log" LISTED="$WORK/listed.log" LOOKED="$WORK/looked.log" \
		GH_TOKEN=x REPO=Glyndor/podup RUN_ID=100 HEAD_SHA="$SHA" EVENT=pull_request HEAD_BRANCH=feature PR_NUMBER=7 INTERVAL=1 WARMUP=2 LIMIT=4 "$@" \
		timeout 8 bash "${S:-$script}" >"$WORK/out.log" 2>&1
	echo $?
}
fresh() { rm -rf "$WORK/looks" && mkdir "$WORK/looks"; }

listing \
	"$(run_json 100 pull_request in_progress $WF/ci.yml)" \
	"$(run_json 101 pull_request in_progress $WF/ci.yml)" \
	"$(run_json 102 pull_request queued $WF/podman-lane.yml)" \
	"$(run_json 103 pull_request in_progress $WF/debian-build.yml)" \
	"$(run_json 104 push in_progress $WF/ci.yml)" \
	"$(run_json 105 merge_group in_progress $WF/ci.yml)" \
	"$(run_json 106 schedule in_progress $WF/podman-lane.yml)" \
	"$(run_json 108 push in_progress $WF/release.yml)" \
	"$(run_json 109 pull_request in_progress $WF/release.yml)" \
	"$(run_json 110 pull_request in_progress $WF/audit.yml)" \
	"$(run_json 111 pull_request in_progress $WF/supply-chain.yml)" \
	"$(run_json 112 pull_request in_progress $WF/commit-hygiene.yml)" \
	"$(run_json 113 pull_request in_progress $WF/dco.yml)" \
	"$(run_json 114 pull_request in_progress $WF/some-new-workflow.yml)" \
	"$(run_json 115 pull_request completed $WF/ci.yml)" \
	"$(run_json 116 pull_request in_progress $WF/ci.yml $OTHER_SHA)" \
	"$(run_json 117 pull_request in_progress $WF/ci.yml $SHA Glyndor/podup other-branch)" \
	"$(run_json 118 pull_request in_progress $WF/ci.yml $SHA someone/podup)" \
	"$(run_json 121 pull_request in_progress $WF/ci.yml $SHA Glyndor/podup feature 8)" \
	"$(run_json 119 pull_request waiting $WF/lint-shell.yml)" \
	"$(run_json 120 pull_request in_progress "$WF/ci.yml@refs/pull/9/merge")"

cases() { # the cases, against the script in $S
	# The first look is quiet and the second has a failure: it cuts at the second.
	fresh
	look_file 1 "$(chk in_progress null 100 'rust / Test')" "$(chk in_progress null 101 'rust / Coverage')" "$(chk completed success 100 'stop-on-failure / stop at the first failure')"
	look_file 2 "$(chk completed failure 100 'rust / Format & lint')" "$(chk in_progress null 101 'rust / Coverage')"
	check "a failure at the second look: the job exits 0" 0 "$(go)"
	check "it cuts exactly the unfinished runs of this commit, event and repository on the list, then its own" "101,102,103,119,120,100" "$(ids)"
	check "...and two looks were enough: one request each" 2 "$(wc -l <"$WORK/looked.log")"
	check "every look asks for the checks of this commit, the latest of each" "2 2" "$(grep -cF "commits/${SHA}/check-runs?per_page=100&filter=latest" "$WORK/looked.log") $(wc -l <"$WORK/looked.log")"
	check "the listing asks for this commit and this event" yes "$([ "$(grep -cF "head_sha=${SHA}&event=pull_request" "$WORK/listed.log")" -ge 1 ] && echo yes || echo no)"
	for kept in 104:"a push run" 105:"a merge group run" 106:"a scheduled run" 108:"a release" 109:"a release called from a pull request" 110:"the audit" 111:"the supply chain" 112:"the commit hygiene" 113:"DCO" 114:"a workflow that is not on the list" 115:"a finished run" 116:"another commit's run" 117:"another branch's run" 118:"another repository's run" 121:"another pull request's run, on the same branch"; do
		id="${kept%%:*}"
		check "it keeps ${kept#*:}" 0 "$(ids | tr ',' '\n' | grep -cx "$id")"
	done
	case "$(cat "$WORK/out.log")" in *"::notice::"*"failed after"*"rust / Format & lint (run 100)"*) check "it says which job failed and when" yes yes ;; *) check "it says which job failed and when" notice "$(cat "$WORK/out.log")" ;; esac

	# timed_out is a failure.
	fresh
	look_file 1 "$(chk completed timed_out 101 'rust / MSRV')"
	check "a timed out job cuts" "0 101,102,103,119,120,100" "$(go) $(ids)"

	# What is not a failure.
	fresh
	look_file 1 "$(chk completed cancelled 101 a)" "$(chk completed skipped 101 b)" "$(chk completed neutral 101 c)" "$(chk completed success 101 'a job allowed to fail')" "$(chk in_progress null 100 'rust / Test')"
	check "cancelled, skipped, neutral and a job allowed to fail are not failures: nothing is cut" "0 none" "$(go) $(ids)"
	fresh
	look_file 1 "$(chk completed failure 101 'codecov/patch' codecov)"
	check "a check that is not a job of Actions is no failure" "0 none" "$(go) $(ids)"

	# A failure of a workflow that is not on the list still cuts the ones that are.
	fresh
	look_file 1 "$(chk completed failure 113 'dco / Signed-off-by present on every commit')"
	check "a failed job of DCO cuts the expensive runs, and DCO is never one of them" "0 101,102,103,119,120,100" "$(go) $(ids)"

	# A failed check says nothing about whose it is: the one of a fork, of another branch, of another pull
	# request, of another event or of a scheduled run of the same commit is nobody's failure here.
	for foreign in 118:"a fork" 117:"another branch" 121:"another pull request on the same branch" 104:"a push" 105:"a merge group" 106:"a scheduled run of the same commit"; do
		fresh
		look_file 1 "$(chk completed failure "${foreign%%:*}" 'rust / Test')" "$(chk in_progress null 101 'rust / Coverage')"
		check "a failed check of ${foreign#*:} cuts nothing" "0 none" "$(go LIMIT=2) $(ids)"
	done
	fresh
	look_file 1 "$(chk completed failure 118 'rust / Test')"
	look_file 2 "$(chk completed failure 118 'rust / Test')" "$(chk completed failure 101 'rust / Lint')"
	check "a foreign failure is skipped and the failure of this pull request that follows still cuts" "0 101,102,103,119,120,100" "$(go) $(ids)"
	fresh
	look_file 1 "$(chk completed failure 118 'rust / Test')" "$(chk in_progress null 101 'rust / Coverage')"
	go LIMIT=4 >/dev/null
	check "a run that is not this one's is looked up once, not at every look" "1" "$(grep -c 'head_sha=' "$WORK/listed.log")"

	# Nothing left to wait for: it ends, but only after the warmup.
	fresh
	look_file 1 "$(chk completed success 100 'rust / Test')" "$(chk completed success 101 'rust / Lint')"
	rc="$(go WARMUP=2)"
	check "every job finished and none failed: ends with nothing cut" "0 none" "$rc $(ids)"
	check "...not before the warmup (three looks at one second), since the other workflows may not have started" "3" "$(wc -l <"$WORK/looked.log")"
	fresh
	look_file 1 "$(chk in_progress null 101 'rust / Test')"
	rc="$(go LIMIT=3 WARMUP=100)"
	check "something still running at the limit: it stops watching, cuts nothing, and succeeds" "0 none" "$rc $(ids)"
	case "$(cat "$WORK/out.log")" in *"limit of 3 s"*) check "...and says it gave up" yes yes ;; *) check "...and says it gave up" limit "$(cat "$WORK/out.log")" ;; esac

	# It never fails the job.
	fresh
	look_file 1 "$(chk completed failure 100 'rust / Test')"
	check "a look that fails is reported and does not fail the job" "0" "$(go LOOK_FAIL=1 LIMIT=2)"
	case "$(cat "$WORK/out.log")" in *"::warning::"*"could not be read"*) check "...with a warning" yes yes ;; *) check "...with a warning" warning "$(cat "$WORK/out.log")" ;; esac
	check "a cancellation that is refused is reported and does not fail the job" "0 none" "$(go CANCEL_FAIL=1) $(ids)"
	case "$(cat "$WORK/out.log")" in *"::warning::"*"could not be cancelled"*) check "...naming the run" yes yes ;; *) check "...naming the run" warning "$(cat "$WORK/out.log")" ;; esac
	check "a listing that cannot be read cuts nothing: a failure that cannot be attributed is nobody's" "0 none" "$(go LIST_FAIL=1 LIMIT=2) $(ids)"
	case "$(cat "$WORK/out.log")" in *"is not attributed"*) check "...and says so" yes yes ;; *) check "...and says so" attribution "$(cat "$WORK/out.log")" ;; esac
	for v in GH_TOKEN= REPO= RUN_ID= HEAD_SHA= EVENT= HEAD_BRANCH=; do
		check "a missing ${v%=} cuts nothing, does not fail the job and does not even look" "0 none 0" "$(go "$v") $(ids) $(wc -l <"$WORK/looked.log")"
	done
	check "a commit that is not a hash cuts nothing" "0 none" "$(go HEAD_SHA='x; rm') $(ids)"
	check "a run id that is not a number cuts nothing" "0 none" "$(go RUN_ID='1;2') $(ids)"
	check "a pull request without its number cuts nothing and does not even look" "0 none 0" "$(go PR_NUMBER=) $(ids) $(wc -l <"$WORK/looked.log")"
	check "a branch with refs/heads/ in front of it cuts nothing" "0 none" "$(go HEAD_BRANCH=refs/heads/feature) $(ids)"
	check "an event that is not a pull request or a merge group cuts nothing" "0 none" "$(go EVENT=schedule) $(ids)"
	fresh
	look_file 1 "$(chk completed failure 105 'rust / Test')"
	check "a merge group is watched, and only its own runs are cut" "0 105" "$(go EVENT=merge_group HEAD_BRANCH=feature PR_NUMBER= RUN_ID=105) $(ids)"
}

echo "== the script"
S="$script" cases

# The workflow side.
judge() { # <script> <reusable> <ci.yml>
	python3 -I - "$1" "$2" "$3" "$HERE/.github/scripts/lib" "$HERE/.github/scripts/ci/cancel-others.sh" <<'PYEOF'
import re
import sys

sys.path.insert(0, sys.argv[4])
import ci_yaml  # noqa: E402

script, reusable, ci, others = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[5]
spec = ci_yaml.load(reusable)
if spec.get("permissions") != {}:
    print("the reusable workflow does not start from no permissions")
job = spec["jobs"]["watch"]
if job.get("permissions") != {"contents": "read", "actions": "write", "checks": "read"}:
    print("the job holds other permissions than contents read, actions write and checks read")
if "pull_request" not in str(job.get("if")) or "merge_group" not in str(job.get("if")) or "schedule" in str(job.get("if")):
    print("the job is not limited to a pull request and the merge queue")
if not job.get("timeout-minutes"):
    print("the job has no timeout")
uses = [s.get("uses") for s in job["steps"] if s.get("uses")]
if any(not str(u).startswith("actions/checkout@") for u in uses):
    print("the job uses an action other than checkout")
if "secrets" in open(reusable).read():
    print("the job reads a secret")
body = next(s["run"] for s in job["steps"] if "stop-on-failure.sh" in str(s.get("run", "")))
if 'git show "${BASE_SHA}:${script}"' not in body or 'bash "$script"' not in body:
    print("a pull request does not run the base commit's copy of the script")
if "exit 0" not in body:
    print("a base without the script does not end quietly")
# the caller
c = ci_yaml.load(ci)
caller = c["jobs"].get("stop-on-failure")
if not caller or caller.get("uses") != "./.github/workflows/reusable-stop-on-failure.yml":
    print("ci.yml does not call the watcher")
else:
    if caller.get("permissions") != {"contents": "read", "actions": "write", "checks": "read"}:
        print("the caller grants other permissions")
    if caller.get("needs") or caller.get("if"):
        print("the caller waits for something or has a condition: it must start with the run")
for name, j in c["jobs"].items():
    needs = j.get("needs")
    needs = [needs] if isinstance(needs, str) else (needs or [])
    if "stop-on-failure" in needs:
        print(f"the job `{name}` waits for the watcher, which has to be nobody's dependency")
# the list
mine = re.search(r"CANCELLABLE=\((.*?)\n\)", open(script).read(), re.S).group(1).split()
theirs = re.search(r"CANCELLABLE=\((.*?)\n\)", open(others).read(), re.S).group(1).split()
if mine != theirs:
    print("the list of workflows it may cut is not the one of cancel-others.sh")
for w in mine:
    if re.search(r"codeql|security|audit|release|publish|supply|dco|hygiene|secret|scorecard|dependabot", w, re.I):
        print(f"the list holds {w}, which is an analysis, a release or a publication")
PYEOF
}
echo "== the workflows"
CI="$HERE/.github/workflows/ci.yml"
check "the watcher is minimal, limited, and nobody's dependency" "" "$(judge "$script" "$reusable" "$CI")"

plant() { # <description> <file: reusable|ci|script> <old, once> <new> <expected fragment>
	local re="$WORK/p-re.yml" ci="$WORK/p-ci.yml" sc="$WORK/p-script.sh" target
	cp "$reusable" "$re"
	cp "$CI" "$ci"
	cp "$script" "$sc"
	case "$2" in reusable) target="$re" ;; ci) target="$ci" ;; *) target="$sc" ;; esac
	if ! python3 - "$target" "$3" "$4" <<'PYEOF'
import sys
path, old, new = sys.argv[1:4]
text = open(path).read()
if text.count(old) != 1:
    sys.exit(3)
open(path, "w").write(text.replace(old, new))
PYEOF
	then
		check "the plant applies: $1" applied not-applied
		return
	fi
	check "plant: $1" 1 "$(judge "$sc" "$re" "$ci" | grep -cF "$5")"
}
plant "the job may write the contents" reusable '      contents: read
      actions: write' '      contents: write
      actions: write' "holds other permissions"
plant "the job reads no checks" reusable '      checks: read
    steps:' '    steps:' "holds other permissions"
plant "the job also runs on the schedule" reusable "github.event_name == 'pull_request' || github.event_name == 'merge_group'" "github.event_name != 'push'" "is not limited"
plant "a third-party action" reusable '      - name: Watch the check runs' '      - uses: some/action@v1
      - name: Watch the check runs' "uses an action other than checkout"
plant "a pull request runs the script of its own tree" reusable '          if git show "${BASE_SHA}:${script}" >"$RUNNER_TEMP/stop.sh" 2>/dev/null; then' '            if cp "$script" "$RUNNER_TEMP/stop.sh"; then' "base commit's copy"
plant "the aggregator waits for the watcher" ci '  deprecations:
    needs: rust' '  deprecations:
    needs: [rust, stop-on-failure]' "waits for the watcher"
plant "the watcher waits for the suite it should cut" ci '    uses: ./.github/workflows/reusable-stop-on-failure.yml' '    needs: rust
    uses: ./.github/workflows/reusable-stop-on-failure.yml' "waits for something"
plant "the caller grants write on the contents" ci '      actions: write
      checks: read
    uses: ./.github/workflows/reusable-stop-on-failure.yml' '      actions: write
      checks: read
      pull-requests: write
    uses: ./.github/workflows/reusable-stop-on-failure.yml' "grants other permissions"
plant "the analysis of the code on the list" script '	.github/workflows/fuzz.yml
)' '	.github/workflows/fuzz.yml
	.github/workflows/codeql.yml
)' "is not the one of cancel-others.sh"
plant "the release on both lists is still refused by name" script '	.github/workflows/fuzz.yml
)' '	.github/workflows/fuzz.yml
	.github/workflows/release.yml
)' "is not the one of cancel-others.sh"

echo "== the sabotages of the script"
SABS=0
sabotage() { # <description> <old text, once> <new text>: started in the background, collected below
	SABS=$((SABS + 1))
	local n="$SABS" copy="$WORK/sabotaged-$SABS.sh"
	echo "$1" >"$WORK/sab-$n.name"
	if ! python3 - "$script" "$copy" "$2" "$3" <<'PYEOF'
import sys
src, dst, old, new = sys.argv[1:5]
text = open(src).read()
if text.count(old) != 1:
    sys.exit(3)
open(dst, "w").write(text.replace(old, new))
PYEOF
	then
		echo notapplied >"$WORK/sab-$n.res"
		return
	fi
	chmod +x "$copy"
	(
		# Its own copy of everything the cases write, so they can run side by side.
		mkdir -p "$WORK/s$n"
		cp -R "$WORK/bin" "$WORK/runs.json" "$WORK/s$n/"
		WORK="$WORK/s$n"
		S="$copy"
		pass=0
		fail=0
		cases >/dev/null 2>&1
		echo "$fail" >"$WORK/../sab-$n.res"
	) &
}
sabotage "a timed out job is not a failure" '[ "$conclusion" = failure ] || [ "$conclusion" = timed_out ] || continue' '[ "$conclusion" = failure ] || continue'
sabotage "a cancelled job is a failure" '[ "$conclusion" = failure ] || [ "$conclusion" = timed_out ] || continue' '[ "$conclusion" = failure ] || [ "$conclusion" = timed_out ] || [ "$conclusion" = cancelled ] || continue'
sabotage "any app counts, not only Actions" 'select((.app.slug // "") == "github-actions")' 'select(true)'
sabotage "the workflows are not checked against the list" '[ "$listed" -eq 0 ]' 'false'
sabotage "the runs of every event are cut" 'event=${event}&per_page=100' 'per_page=100'
sabotage "this run is not cancelled" 'if gh api -X POST "repos/${repo}/actions/runs/${own}/cancel"' 'if false'
sabotage "its own run is cancelled first" 'cut_runs() {' 'cut_runs() { gh api -X POST "repos/${repo}/actions/runs/${own}/cancel" >/dev/null 2>&1;'
sabotage "it ends before the other workflows had time to start" '"$elapsed" -ge "$warmup" ]' 'true ]'
sabotage "the run of another repository is counted and cut" 'and (.head_repository.full_name // "") == env.REPO' ''
sabotage "the run of another branch is counted and cut" 'and (.head_branch // "") == env.BRANCH' ''
sabotage "the run of another event is counted and cut" 'select(.head_sha == env.SHA and .event == env.EV' 'select(.head_sha == env.SHA'
sabotage "the run of another commit is counted and cut" 'select(.head_sha == env.SHA and .event == env.EV' 'select(.event == env.EV'
sabotage "the run of another pull request is counted and cut" 'and (env.EV != "pull_request" or any(.pull_requests[]?; (.number | tostring) == env.PR)))' ')'
sabotage "a failed check is attributed to whoever listed it" '					notmine="$notmine$run "
					continue' '					notmine="$notmine$run "'
sabotage "a failed check is believed before it is attributed" 'if ! printf '"'"'%s\n'"'"' "$mine" | grep -qx "$run"; then
				if listed=' 'if false; then
				if listed='
sabotage "a branch is not required" '|| [ -z "$branch" ]; then' '; then'
sabotage "a pull request is watched without its number" '	case "$pr" in *[!0-9]* | "") echo "::warning::stop-on-failure: a pull request without a number; nothing is watched"; exit 0 ;; esac' '	:'
sabotage "a refused cancellation fails the job" 'echo "::warning::stop-on-failure: run $id of $path could not be cancelled"' 'exit 1'
sabotage "no limit to the watching" 'if [ "$elapsed" -ge "$limit" ]; then' 'if false; then'

wait
for i in $(seq 1 "$SABS"); do
	r="$(cat "$WORK/sab-$i.res" 2>/dev/null || echo missing)"
	case "$r" in
	notapplied) check "the sabotage applies: $(cat "$WORK/sab-$i.name")" applied not-applied ;;
	*) check "sabotage: $(cat "$WORK/sab-$i.name")" yes "$([ "$r" != missing ] && [ "$r" -gt 0 ] && echo yes || echo no)" ;;
	esac
done

echo
echo "passed: $pass  failed: $fail"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
