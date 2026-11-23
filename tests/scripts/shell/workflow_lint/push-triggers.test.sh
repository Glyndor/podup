#!/usr/bin/env bash
#
# .github/scripts/checks/check-push-triggers.py: the merge queue runs a workflow on the
# commit that lands in develop, so a workflow with a push to develop either does not
# exist (when it is also in the queue) or is on a short list with its reason. Each rule is
# attacked in both directions: a change that must be refused, and the change just inside
# it that must pass.
#
# Requires: python3 with PyYAML.
# shellcheck disable=SC2016
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
GATE="$HERE/.github/scripts/checks/check-push-triggers.py"
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
	rm -rf "$WORK/t"
	mkdir -p "$WORK/t/.github"
	cp -r "$HERE/.github/workflows" "$WORK/t/.github/workflows"
}
verdict() { python3 "$GATE" "$WORK/t" >"$WORK/out" 2>&1; echo $?; }
says() { grep -c -- "$1" "$WORK/out"; }
edit() { # <file> <python expression over s>
	python3 - "$WORK/t/.github/workflows/$1" "$2" <<'PY'
import sys
path, expr = sys.argv[1], sys.argv[2]
s = open(path).read()
t = eval(expr)
assert t != s, "the change did nothing"
open(path, "w").write(t)
PY
}

check "the real workflows pass" 0 "$(python3 "$GATE" "$HERE" >/dev/null 2>&1; echo $?)"

# Rule 1: in the queue and on a push to develop is a second run of the same commit.
setpush() { # <file> <python expression for the push value>
	python3 - "$WORK/t/.github/workflows/$1" "$2" <<'PY'
import sys, yaml
path, value = sys.argv[1], eval(sys.argv[2])
spec = yaml.safe_load(open(path))
key = "on" if "on" in spec else True
spec[key]["push"] = value
open(path, "w").write(yaml.safe_dump(spec, sort_keys=False))
PY
}
for w in ci.yml line-limit.yml workflow-lint.yml audit.yml commit-hygiene.yml dco.yml; do
	fresh
	setpush "$w" '{"branches": ["main", "develop"]}'
	check "$w: a push to develop beside the queue is refused" 1 "$(verdict)"
done
fresh
setpush ci.yml '{"branches": ["main"]}'
check "ci.yml: the push to main alone passes" 0 "$(verdict)"
fresh
setpush ci.yml '{"branches": ["main", "develop"]}'
check "ci.yml: develop added to the push branches is refused" 1 "$(verdict)"
check "...and the message says why" 1 "$(says 'runs twice on the commit that lands')"
fresh
setpush ci.yml 'None'
check "ci.yml: a push with no branches filter reaches develop and is refused" 1 "$(verdict)"
fresh
setpush ci.yml '{"branches-ignore": ["main"]}'
check "ci.yml: a push that ignores main reaches develop and is refused" 1 "$(verdict)"
fresh
setpush ci.yml '{"branches-ignore": ["develop"]}'
check "ci.yml: a push that ignores develop does not reach it" 0 "$(verdict)"
fresh
setpush ci.yml '{"branches": "develop"}'
check "ci.yml: a push to develop written as a string is refused" 1 "$(verdict)"
fresh
setpush ci.yml '{"branches": ["main"], "tags": ["v*"]}'
check "ci.yml: a push to main and to tags is not a push to develop" 0 "$(verdict)"
fresh
setpush ci.yml '{"branches": ["release/**"]}'
check "ci.yml: a push to another branch is not a push to develop" 0 "$(verdict)"
fresh
setpush ci.yml '{"tags": ["v*"]}'
check "ci.yml: a push to tags only is not a push to develop" 0 "$(verdict)"
fresh
edit audit.yml 's.replace("  merge_group:\n    types: [checks_requested]\n", "  merge_group:\n    types: [checks_requested]\n  push:\n    branches:\n      - develop\n")'
check "audit.yml: the push that was removed, put back, is refused" 1 "$(verdict)"

# Rule 2: a workflow with a push to develop that is not on the list.
fresh
cat >"$WORK/t/.github/workflows/new-suite.yml" <<'YML'
name: new suite
on:
  push:
    branches: [develop]
jobs:
  a:
    runs-on: ubuntu-latest
    steps:
      - run: true
YML
check "a new workflow with a push to develop and no reason is refused" 1 "$(verdict)"
check "...and the message names it" 1 "$(says 'new-suite.yml')"
fresh
cat >"$WORK/t/.github/workflows/new-suite.yaml" <<'YML'
name: new suite
on: push
jobs:
  a:
    runs-on: ubuntu-latest
    steps:
      - run: true
YML
check "a .yaml workflow with a bare push is refused too" 1 "$(verdict)"
fresh
cat >"$WORK/t/.github/workflows/new-suite.yml" <<'YML'
name: new suite
on: [push, pull_request]
jobs:
  a:
    runs-on: ubuntu-latest
    steps:
      - run: true
YML
check "a workflow whose on is a list with push is refused" 1 "$(verdict)"
fresh
cat >"$WORK/t/.github/workflows/new-suite.yml" <<'YML'
name: new suite
on:
  pull_request:
  merge_group:
    types: [checks_requested]
jobs:
  a:
    runs-on: ubuntu-latest
    steps:
      - run: true
YML
check "a new workflow in the queue with no push passes" 0 "$(verdict)"
fresh
cat >"$WORK/t/.github/workflows/new-suite.yml" <<'YML'
name: new suite
on:
  push:
    branches: [main]
    tags: ["v*"]
jobs:
  a:
    runs-on: ubuntu-latest
    steps:
      - run: true
YML
check "a workflow that pushes to main and tags only passes" 0 "$(verdict)"
fresh
printf 'on: [push\n' >"$WORK/t/.github/workflows/broken.yml"
check "a workflow that cannot be read fails" 1 "$(verdict)"
fresh
printf 'on:\n  push:\n    branches: [develop]\n  push:\n    branches: [main]\njobs: {}\n' >"$WORK/t/.github/workflows/dup.yml"
check "a duplicate key cannot hide a push to develop" 1 "$(verdict)"

# The reasons are not empty.
check "an entry of the list with no reason is refused" 1 "$(python3 - "$HERE/.github/scripts/checks" "$HERE" <<'PY' >/dev/null 2>&1; echo $?
import importlib.util, sys
spec = importlib.util.spec_from_file_location("gate", sys.argv[1] + "/check-push-triggers.py")
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)
gate.ALLOWED["lint-shell.yml"] = "  "
sys.exit(1 if any("no reason" in f for f in gate.findings(sys.argv[2])) else 0)
PY
)"

# Rule 3: an entry of the list that no longer holds.
fresh
edit lint-shell.yml 's.replace("  push:\n    branches:\n      - develop\n", "  push:\n    branches:\n      - main\n")'
check "an allowed workflow that lost its push is a stale entry" 1 "$(verdict)"
check "...and the message says stale" 1 "$(says 'stale entry')"
fresh
rm "$WORK/t/.github/workflows/supply-chain.yml"
check "an allowed workflow that no longer exists is a stale entry" 1 "$(verdict)"
fresh
setpush lint-shell.yml '{"branches": ["develop"]}'
python3 - "$WORK/t/.github/workflows/lint-shell.yml" <<'PY'
import sys, yaml
spec = yaml.safe_load(open(sys.argv[1]))
key = "on" if "on" in spec else True
spec[key]["merge_group"] = {"types": ["checks_requested"]}
open(sys.argv[1], "w").write(yaml.safe_dump(spec, sort_keys=False))
PY
check "an allowed workflow that also joins the queue is refused: the push repeats it" 1 "$(verdict)"

echo
echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
