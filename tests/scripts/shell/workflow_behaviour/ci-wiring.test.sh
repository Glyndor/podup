#!/usr/bin/env bash
#
# The wiring of the pipelines that decide what runs and what is certified is
# held in its exact form, not by the presence of its parts.
#
# ci_wiring/shape.py compares every job of the listed workflows with a snapshot:
# the set of jobs is closed, and each job's `needs`, `if`, `with` and steps are
# the ones recorded. Each case below plants one way to loosen the pipeline and
# expects the comparison to go red. The real tree is expected to match.
#
# The cases are the ways to loosen the pipeline, one by one:
#   - the `run:` (or the `if`) of two suites crossed;
#   - a suite handed a fixed `false`;
#   - the plan variables of two suites crossed in the gate;
#   - the gate reading the result of another job for a suite;
#   - the plan script read from the checkout instead of the base commit;
#   - an input that turns a check off passed to a reusable;
#   - a job added, which no list exempts by name;
#   - a structure or hygiene step put behind the plan.
#
# Requires: python3 with PyYAML.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
shape="$HERE/tests/scripts/shell/workflow_behaviour/ci_wiring/shape.py"
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

fresh() {
	rm -rf "$WORK/tree"
	mkdir -p "$WORK/tree/.github"
	cp -r "$HERE/.github/workflows" "$WORK/tree/.github/workflows"
}
verdict() { WIRING_ROOT="$WORK/tree" python3 "$shape" --check >"$WORK/out.log" 2>&1; echo $?; }
mentions() { grep -c -- "$1" "$WORK/out.log"; }
edit() { # <file> <python expression over s>
	python3 - "$WORK/tree/.github/workflows/$1" "$2" <<'PY'
import sys
path, expr = sys.argv[1], sys.argv[2]
s = open(path).read()
t = eval(expr)
assert t != s, "the plant changed nothing"
open(path, "w").write(t)
PY
}
# sabotage <description> <file> <python expression>: the comparison must go red.
sabotage() {
	fresh
	edit "$2" "$3"
	check "$1" 1 "$(verdict)"
}

check "the real tree matches the snapshot" 0 "$(python3 "$shape" --check >/dev/null 2>&1; echo $?)"
fresh
check "a copy of the tree matches the snapshot" 0 "$(verdict)"

sabotages="$HERE/tests/scripts/shell/workflow_behaviour/ci_wiring/sabotage.py"
count="$(python3 "$sabotages" --count)"
check "the list of sabotages is not empty" yes "$([ "$count" -ge 30 ] && echo yes || echo no)"
i=0
while [ "$i" -lt "$count" ]; do
	fresh
	python3 "$sabotages" "$i" "$WORK/tree" || check "sabotage $i finds its text" planted "not found"
	check "caught: $(python3 "$sabotages" --describe "$i")" 1 "$(verdict)"
	i=$((i + 1))
done
R=reusable-rust-ci.yml

# The required names: one workflow, and only the runs that should produce them.
names="$HERE/tests/scripts/shell/workflow_behaviour/ci_wiring/required_names.py"
names_verdict() { WIRING_ROOT="$WORK/tree" python3 "$names" >"$WORK/out.log" 2>&1; echo $?; }
fresh
check "no required name comes from more than one workflow or kind of run" 0 "$(names_verdict)"
i=0
count="$(python3 "$sabotages" names --count)"
while [ "$i" -lt "$count" ]; do
	fresh
	python3 "$sabotages" names "$i" "$WORK/tree" || check "name plant $i finds its text" planted "not found"
	check "caught: $(python3 "$sabotages" names --describe "$i")" 1 "$(names_verdict)"
	i=$((i + 1))
done

# The messages say where.
fresh
j=0
while [ "$(python3 "$sabotages" --describe "$j")" != "coverage turned off in the caller" ]; do j=$((j + 1)); done
python3 "$sabotages" "$j" "$WORK/tree"
verdict >/dev/null
check "the message names the input that changed" 1 "$(mentions 'coverage-threshold')"

# A step condition on the four structure and hygiene steps of lint is absent in the real tree.
check "no structure or hygiene step of lint has a condition" 0 "$(python3 - "$HERE/.github/workflows/$R" <<'PY'
import sys
import yaml

spec = yaml.safe_load(open(sys.argv[1]))
names = ("no planning-note references", "no vetoed vocabulary", "no private document references", "folders hold at most twenty files")
bad = [s["name"] for s in spec["jobs"]["lint"]["steps"] if s.get("name") in names and "if" in s]
missing = [n for n in names if n not in [s.get("name") for s in spec["jobs"]["lint"]["steps"]]]
print(len(bad) + len(missing))
PY
)"

echo
echo "passed: $pass  failed: $fail"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
