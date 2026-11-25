#!/usr/bin/env bash
#
# Behaviour of `rust / Gate`, the one job that certifies the Rust pipeline.
#
# GitHub counts a required check that ended `skipped` as passed, so a suite
# skipped because the plan failed, or because something before it broke, would
# leave a pull request green with nothing run. The gate runs on every outcome
# and fails unless the plan succeeded and every suite either succeeded or was
# skipped for a reason somebody said out loud: an explicit `false` from the
# plan, or a suite the caller switched off.
#
# The step is read out of the workflow and executed as it ships, so the test
# cannot drift from the file. Each rule has a case that breaks it and a case
# just inside it that must stay green. Every `needs` that is cancelled, failed
# or skipped without a reason is red.
#
# Requires: python3 with PyYAML.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
workflow="$HERE/.github/workflows/reusable-rust-ci.yml"
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

python3 - "$workflow" >"$WORK/gate.sh" <<'PY'
import sys
import yaml

spec = yaml.safe_load(open(sys.argv[1]))
steps = [s for s in spec["jobs"]["gate"]["steps"] if s.get("name") == "Certify the plan and every skip"]
assert len(steps) == 1, "the gate has one step that certifies"
print(steps[0]["run"])
PY
check "the gate step was read from the workflow" yes "$([ -s "$WORK/gate.sh" ] && echo yes || echo no)"

# gate <plan> <run> <docs_only> <build> <lint> <test> <extra> <coverage> <msrv> <doc>
# Every suite is switched on and package and semver are off, as in ci.yml.
gate() {
	local needs
	needs="$(CI_ONLY="${CI_ONLY:-false}" python3 - "$@" <<'PY'
import json
import os
import sys

plan, run, docs, build, lint, test, extra, cov, msrv, doc = sys.argv[1:11]
needs = {
    "classify": {"result": plan, "outputs": {"run": run, "docs_only": docs, "ci_only": os.environ["CI_ONLY"], "build": build}},
    "lint": {"result": lint},
    "test": {"result": test},
    "test-extra": {"result": extra},
    "coverage": {"result": cov},
    "msrv": {"result": msrv},
    "doc": {"result": doc},
    "package": {"result": "skipped"},
    "semver": {"result": "skipped"},
}
print(json.dumps(needs))
PY
)"
	env NEEDS="${NEEDS_OVERRIDE:-$needs}" BASE="${BASE:-none}" \
		EXTRA_ON="${EXTRA_ON:-true}" COVERAGE_ON="${COVERAGE_ON:-true}" MSRV_ON="${MSRV_ON:-true}" \
		DOC_ON="${DOC_ON:-true}" PACKAGE_ON="${PACKAGE_ON:-false}" SEMVER_ON="${SEMVER_ON:-false}" \
		bash "$WORK/gate.sh" >"$WORK/out.log" 2>&1
	echo $?
}
green() { check "$1" 0 "$(shift; gate "$@")"; }
red() { check "$1" 1 "$(shift; gate "$@")"; }
s=success
k=skipped

# The whole pipeline ran.
green "everything ran and passed" $s true false true $s $s $s $s $s $s
# A suite skipped because the plan said so: a range that touches nothing the lanes read.
green "plan said run=false: tests, coverage and matrix skipped, build skipped" $s false false false $s $k $k $k $k $k
# A documentation-only range: the focused lane ran, coverage and matrix skipped.
green "plan said docs only: coverage and matrix skipped, tests ran" $s true true false $s $s $k $k $k $k
# Nothing of the build changed: msrv and doc skipped on an explicit build=false.
green "plan said build=false: msrv and doc skipped" $s true false false $s $s $s $s $k $k

# Point 6 and 10: a failed plan turns the gate red, whatever else happened.
red "a failed plan is red even when every suite passed" failure true false true $s $s $s $s $s $s
red "a failed plan with every suite skipped is red" failure "" "" "" $s $k $k $k $k $k
red "a cancelled plan is red" cancelled true false true $s $s $s $s $s $s
red "a skipped plan is red" skipped true false true $s $s $s $s $s $s

# A suite skipped without the plan having said so.
red "tests skipped while the plan said to run them" $s true false true $s $k $s $s $s $s
red "coverage skipped while the plan said to run it" $s true false true $s $s $s $k $s $s
red "the matrix skipped while the plan said to run it" $s true false true $s $s $k $s $s $s
red "msrv skipped while the build answer was true" $s true false true $s $s $s $s $k $s
red "doc skipped while the build answer was true" $s true false true $s $s $s $s $s $k
red "lint is never a legitimate skip" $s false false false $k $k $k $k $k $k
# Point 11: only an explicit "no" skips; a missing or empty answer does not.
red "tests skipped on an empty run answer" $s "" "" "" $s $k $k $k $k $k
red "msrv skipped on an empty build answer" $s false false "" $s $k $k $k $k $k
red "tests skipped on a run answer that is neither true nor false" $s maybe false false $s $k $k $k $k $k
# A suite skipped because something before it failed is not a skip the plan gave.
red "tests skipped after lint failed" $s true false true failure $k $k $k $k $k
red "coverage and matrix skipped after tests failed" $s true false true $s failure $k $k $s $s
# docs_only certifies a skip only as `run=true` plus `docs_only=true`, nothing looser.
red "coverage skipped on docs_only=true without run=true" $s "" true false $s $s $k $k $k $k
red "coverage skipped on an empty docs_only answer" $s true "" false $s $s $k $k $k $k
red "coverage skipped on docs_only=false" $s true false false $s $s $k $k $k $k
# The CI-only answer certifies the same two skips, and only with run=true.
check "coverage and matrix skipped on a CI-only range are green" 0 "$(CI_ONLY=true gate $s true false false $s $s $k $k $s $s)"
check "coverage skipped on ci_only without run=true is red" 1 "$(CI_ONLY=true gate $s "" false false $s $s $k $k $s $s)"
check "coverage skipped on ci_only=false is red" 1 "$(CI_ONLY=false gate $s true false false $s $s $k $k $s $s)"
check "ci_only does not excuse tests" 1 "$(CI_ONLY=true gate $s true false false $s $k $k $k $s $s)"
check "ci_only does not excuse msrv" 1 "$(CI_ONLY=true gate $s true false true $s $s $k $k $k $s)"
check "ci_only is void after a failed plan" 1 "$(CI_ONLY=true gate failure true false false $s $s $k $k $s $s)"
# docs_only means nothing once the plan itself did not succeed.
red "docs-only skips are void when the plan failed" failure true true true $s $s $k $k $k $k

# Any needs that failed or was cancelled is red.
for suite in lint test extra coverage msrv doc; do
	for result in failure cancelled; do
		args=(true false true "$s" "$s" "$s" "$s" "$s" "$s")
		case "$suite" in
			lint) args[3]=$result ;;
			test) args[4]=$result ;;
			extra) args[5]=$result ;;
			coverage) args[6]=$result ;;
			msrv) args[7]=$result ;;
			doc) args[8]=$result ;;
		esac
		red "$suite $result is red" $s "${args[@]}"
	done
done
red "an unknown result is red" $s true false true $s $s $s $s weird $s

# A suite the caller switched off may be skipped; and a switched-off one that failed is still red.
check "coverage switched off may be skipped" 0 "$(COVERAGE_ON=false gate $s true false true $s $s $s $k $s $s)"
check "the matrix switched off may be skipped" 0 "$(EXTRA_ON=false gate $s true false true $s $s $k $s $s $s)"
check "msrv switched off may be skipped" 0 "$(MSRV_ON=false gate $s true false true $s $s $s $s $k $s)"
check "doc switched off may be skipped" 0 "$(DOC_ON=false gate $s true false true $s $s $s $s $s $k)"
check "a package check that is on but skipped is red" 1 \
	"$(PACKAGE_ON=true gate $s true false true $s $s $s $s $s $s)"
check "a semver check that is on but skipped is red" 1 \
	"$(SEMVER_ON=true gate $s true false true $s $s $s $s $s $s)"
check "a switched-off suite that failed is red" 1 "$(COVERAGE_ON=false gate $s true false true $s $s $s failure $s $s)"
# An empty switch is not an explicit "no": the suite is treated as on.
check "an empty switch does not excuse a skip" 1 "$(COVERAGE_ON="" gate $s true false true $s $s $s $k $s $s)"

# The messages name what went wrong.
gate failure true false true $s $s $s $s $s $s >/dev/null
check "a failed plan is named" 1 "$(grep -c 'the plan did not succeed (failure)' "$WORK/out.log")"
gate $s true false true $s $k $s $s $s $s >/dev/null
check "an uncertified skip is named" 1 "$(grep -c 'test was skipped and nothing certifies the skip' "$WORK/out.log")"
gate $s true false true $s $s $s $s cancelled $s >/dev/null
check "a cancelled needs is named" 1 "$(grep -c 'msrv: cancelled' "$WORK/out.log")"

# The table: the plan's answers and each job's result, printed where a reader
# looks first, so "it ran" and "everything inside was skipped" are told apart.
gate $s true true false $s $s $k $k $k $k >/dev/null
check "the table prints the plan's answers" 1 "$(grep -c "^plan: success, run='true', docs_only='true', ci_only='false', build='false'" "$WORK/out.log")"
check "the table prints a job that ran" 1 "$(grep -cE '^test +success +ran$' "$WORK/out.log")"
check "the table prints a certified skip" 1 "$(grep -cE '^coverage +skipped +skipped, the plan said so$' "$WORK/out.log")"
gate $s true false true $s $k $k $k $s $s >/dev/null
check "the table prints a skip nothing certifies" 1 "$(grep -cE '^test +skipped +SKIPPED WITHOUT A REASON$' "$WORK/out.log")"
# The base is checked exactly, in shell: `==` in a name expression ignores case.
check "a pull request into develop is certified" 0 "$(BASE=develop gate $s true false true $s $s $s $s $s $s)"
check "a pull request into main is certified" 0 "$(BASE=main gate $s true false true $s $s $s $s $s $s)"
check "a base that differs from develop by case is refused" 1 "$(BASE=Develop gate $s true false true $s $s $s $s $s $s)"
check "a base that differs from main by case is refused" 1 "$(BASE=MAIN gate $s true false true $s $s $s $s $s $s)"
check "any other base is refused" 1 "$(BASE=feature gate $s true false true $s $s $s $s $s $s)"
# What the gate reads is the result of the job itself, and a job that is not there is red.
check "a needs that is not JSON is red" 1 "$(NEEDS_OVERRIDE='not json' gate $s true false true $s $s $s $s $s $s)"
check "a needs with no entry for the tests is red" 1 \
	"$(NEEDS_OVERRIDE='{"classify":{"result":"success","outputs":{"run":"true"}},"lint":{"result":"success"}}' gate $s true false true $s $s $s $s $s $s)"

# A pull request into main promotes what develop holds: no plan narrows it. The
# plan steps are run as shipped, with a git that records what it was asked.
mkdir -p "$WORK/fake"
cat >"$WORK/fake/git" <<'GIT'
#!/usr/bin/env bash
echo "git $*" >>"$GIT_LOG"
exit 1
GIT
chmod +x "$WORK/fake/git"
plan_step() { # <workflow> <job> <step id> <BASE_REF> -> the outputs, one line, and whether git was asked
	python3 - "$HERE/.github/workflows/$1" "$2" "$3" >"$WORK/step.sh" <<'PY'
import sys
import yaml

spec = yaml.safe_load(open(sys.argv[1]))
step = next(s for s in spec["jobs"][sys.argv[2]]["steps"] if s.get("id") == sys.argv[3])
print(step["run"])
PY
	: >"$WORK/gh-out"
	: >"$WORK/git.log"
	env PATH="$WORK/fake:$PATH" GIT_LOG="$WORK/git.log" GITHUB_OUTPUT="$WORK/gh-out" BASE_REF="$4" EVENT="${EVENT:-pull_request}" \
		BASE_SHA=base HEAD_SHA=head bash "$WORK/step.sh" >/dev/null 2>&1
	printf '%s|git:%s' "$(paste -sd, "$WORK/gh-out")" "$(wc -l <"$WORK/git.log" | tr -d ' ')"
}
check "a pull request into main runs every lane, with no judge asked" \
	"run=true,docs_only=false,ci_only=false|git:0" "$(plan_step reusable-rust-ci.yml classify decide main)"
check "a pull request into main builds, with no judge asked" "build=true|git:0" "$(plan_step reusable-rust-ci.yml classify build main)"
check "a pull request into a base that is not exactly develop runs every lane" \
	"run=true,docs_only=false,ci_only=false|git:0" "$(plan_step reusable-rust-ci.yml classify decide Develop)"
check "a pull request into another base runs every lane" \
	"run=true,docs_only=false,ci_only=false|git:0" "$(plan_step reusable-rust-ci.yml classify decide feature)"
check "a pull request into develop asks the judge" "|git:1" "$(plan_step reusable-rust-ci.yml classify decide develop)"
check "the build answer for a pull request into develop asks git" "build=true|git:1" "$(plan_step reusable-rust-ci.yml classify build develop)"
# The queue asks the judge like a pull request into develop does: nothing is skipped on
# the word of another run.
check "the queue asks the judge for the plan" "|git:1" "$(EVENT=merge_group plan_step reusable-rust-ci.yml classify decide "")"
check "the queue asks git for the build answer" "build=true|git:1" "$(EVENT=merge_group plan_step reusable-rust-ci.yml classify build "")"
check "the workflow lint runs for a pull request into main, with no judge asked" "run=true|git:0" "$(plan_step reusable-workflow-lint.yml workflow-lint decide main)"
check "the workflow lint runs for a base that is not exactly develop" "run=true|git:0" "$(plan_step reusable-workflow-lint.yml workflow-lint decide Develop)"
check "the workflow lint asks the judge for a pull request into develop" "|git:1" "$(plan_step reusable-workflow-lint.yml workflow-lint decide develop)"

# Wiring: the gate waits for every job it certifies, runs on every outcome, and
# carries the name the ruleset requires.
wiring="$(python3 - "$workflow" <<'PY'
import sys
import yaml

spec = yaml.safe_load(open(sys.argv[1]))
jobs = spec["jobs"]
gate = jobs["gate"]
problems = []
if gate.get("if") != "always()":
    problems.append("gate-if")
if not str(gate.get("name")).startswith("Gate"):
    problems.append("gate-name")
certified = [j for j in jobs if j != "gate"]
for job in certified:
    if job not in gate["needs"]:
        problems.append("needs-" + job)
for job in ("lint", "test", "test-extra", "coverage", "msrv", "doc"):
    if "!cancelled()" not in str(jobs[job].get("if")):
        problems.append("cancelled-" + job)
print(",".join(problems) or "ok")
PY
)"
check "the gate waits for every job and each suite starts when the plan failed" ok "$wiring"

# Sabotage: a gate without `if: always()` is skipped when a suite fails, and a
# skipped required check reports success. The wiring check above must notice.
sed '/^  gate:/,$ s/^    if: always()$/    if: success()/' "$workflow" >"$WORK/sabotaged.yml"
check "the sabotaged copy differs from the original" 1 "$(cmp -s "$workflow" "$WORK/sabotaged.yml" && echo 0 || echo 1)"
broken="$(python3 - "$WORK/sabotaged.yml" <<'PY'
import sys
import yaml

spec = yaml.safe_load(open(sys.argv[1]))
print("detected" if spec["jobs"]["gate"].get("if") != "always()" else "missed")
PY
)"
check "a gate without always() is noticed" detected "$broken"

echo
echo "passed: $pass  failed: $fail"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
