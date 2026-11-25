#!/usr/bin/env bash
#
# Every workflow the wiring contract holds (FILES in ci_wiring/shape.py) is in both `paths:`
# lists of lint-shell.yml, the one of the pull request and the one of the push. The contract
# test runs from that workflow, so a change to a held workflow that the lists leave out
# reaches the merge without the test that holds it. The test also attacks itself: a held file
# taken out of either list is found, and a file added to the contract and to neither is found.
#
# Requires: python3 with PyYAML.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
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

# <workflow file> <extra held files, space separated>: the held files that a list leaves out,
# one `event: path` per line.
missing() {
	python3 - "$HERE" "$1" "${2:-}" <<'PY'
import os, sys
here, workflow, extra = sys.argv[1], sys.argv[2], sys.argv[3].split()
sys.path.insert(0, os.path.join(here, ".github", "scripts", "lib"))
sys.path.insert(0, os.path.join(here, "tests", "scripts", "shell", "workflow_behaviour", "ci_wiring"))
import ci_yaml, shape
spec = ci_yaml.load(workflow)
on = spec.get("on", spec.get(True))
for event in ("pull_request", "push"):
    paths = set(on[event]["paths"])
    for name in list(shape.FILES) + extra:
        if ".github/workflows/" + name not in paths:
            print(f"{event}: {name}")
PY
}

real="$HERE/.github/workflows/lint-shell.yml"
check "every held workflow is in both lists of lint-shell.yml" "" "$(missing "$real")"

for list in pull_request push; do
	python3 - "$real" "$WORK/$list.yml" "$list" <<'PY'
import sys
src, out, event = sys.argv[1:4]
lines = open(src).read().split("\n")
section = None
kept = []
done = False
for line in lines:
    if line.startswith("  pull_request:"):
        section = "pull_request"
    elif line.startswith("  push:"):
        section = "push"
    elif line and not line.startswith(" "):
        section = None
    if section == event and not done and line.strip() == '- ".github/workflows/audit.yml"':
        done = True
        continue
    kept.append(line)
assert done, "the plant found nothing to remove"
open(out, "w").write("\n".join(kept))
PY
	check "audit.yml taken out of the $list list is found" "$list: audit.yml" "$(missing "$WORK/$list.yml")"
done
check "a workflow added to the contract and to no list is found" "$(printf 'pull_request: brand-new.yml\npush: brand-new.yml')" "$(missing "$real" brand-new.yml)"

echo
echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
