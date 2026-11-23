#!/usr/bin/env bash
#
# The rule that holds `actions: write` to an exact list of steps.
#
# The permission can end the runs of any workflow, so a job that holds it may
# run only the steps check-write-token-jobs.py lists, and nothing in them may
# run a file from the pull request's tree. Each case plants one way to hold the
# permission or to widen what the job runs, and expects the rule to refuse it;
# the real tree is expected to pass.
#
# Requires: python3 with PyYAML.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
checker="$HERE/.github/scripts/checks/check-write-token-jobs.py"
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

# fresh: a copy of the real workflows to plant against.
fresh() {
	rm -rf "$WORK/tree"
	mkdir -p "$WORK/tree/.github"
	cp -r "$HERE/.github/workflows" "$WORK/tree/.github/workflows"
}
# verdict: the exit status of the rule over the planted tree.
verdict() {
	(cd "$WORK/tree" && python3 "$checker" >"$WORK/out.log" 2>&1)
	echo $?
}
message() { grep -c -- "$1" "$WORK/out.log"; }
edit() { # <file> <python expression over s>: rewrite a planted workflow
	python3 - "$WORK/tree/.github/workflows/$1" "$2" <<'PY'
import sys
path, expr = sys.argv[1], sys.argv[2]
s = open(path).read()
t = eval(expr)
assert t != s, "the plant changed nothing"
open(path, "w").write(t)
PY
}

fresh
check "the real tree passes" 0 "$(verdict)"

# Holding the permission outside the one job.
fresh
edit ci.yml "s.replace('permissions:\n  contents: read\n', 'permissions:\n  contents: read\n  actions: write\n', 1)"
check "actions: write at the top of a workflow is refused" 1 "$(verdict)"
check "...and says it belongs to one job" 1 "$(message 'belongs to one job')"

fresh
edit ci.yml "s.replace('permissions:\n  contents: read\n', 'permissions: write-all\n', 1)"
check "write-all at the top of a workflow is refused" 1 "$(verdict)"

fresh
edit lint-shell.yml "s.replace('jobs:\n  shellcheck:\n', 'jobs:\n  shellcheck:\n    permissions: write-all\n', 1)"
check "write-all on a job that calls a reusable is refused" 1 "$(verdict)"
check "...because the reusable is not callable with it" 1 "$(message 'not in CALLABLE')"

plant() { # <perms block as YAML lines at job indent>: a new workflow with one job holding it
	python3 - "$WORK/tree/.github/workflows/planted.yml" "$1" <<'PY'
import sys
open(sys.argv[1], "w").write(
    "name: planted\non:\n  pull_request:\njobs:\n  ship:\n    runs-on: ubuntu-latest\n"
    + sys.argv[2]
    + "    steps:\n      - run: echo hi\n"
)
PY
}
fresh
plant $'    permissions: write-all\n'
check "write-all on a job with steps is refused" 1 "$(verdict)"
# shellcheck disable=SC2016
check "...as a job that is not in the list" 1 "$(message 'is not in KNOWN')"
fresh
plant $'    permissions:\n      actions: write\n'
check "actions: write on a job with steps is refused" 1 "$(verdict)"
fresh
plant $'    permissions:\n      contents: read\n'
check "a job with other permissions passes" 0 "$(verdict)"

# Inheriting it. A job with no permissions of its own gets its workflow's.
fresh
edit audit.yml "s.replace('  audit-develop:\n', '  audit-develop:\n', 1).replace('permissions:\n  contents: read\n', 'permissions:\n  contents: read\n  actions: write\n', 1)"
check "a workflow-level grant that the nightly launcher would inherit is refused" 1 "$(verdict)"

# Inheriting it through a reusable workflow that declares no permissions.
fresh
python3 - "$WORK/tree/.github/workflows" <<'PY'
import sys
d = sys.argv[1]
open(d + "/reusable-plain.yml", "w").write(
    "name: plain\non:\n  workflow_call:\njobs:\n  work:\n    runs-on: ubuntu-latest\n"
    "    steps:\n      - run: bash scripts/x.sh\n"
)
open(d + "/caller-plain.yml", "w").write(
    "name: caller\non:\n  pull_request:\njobs:\n  call:\n    permissions:\n      actions: write\n"
    "    uses: ./.github/workflows/reusable-plain.yml\n"
)
PY
check "a reusable with no permissions inherits what its caller holds, and is refused" 1 "$(verdict)"
check "...naming the caller's call to a reusable that is not callable" 1 "$(message 'not in CALLABLE')"
rm -f "$WORK/tree/.github/workflows/caller-plain.yml"
check "the same reusable called without the permission passes" 0 "$(verdict)"
python3 - "$WORK/tree/.github/workflows" <<'PY'
import sys
open(sys.argv[1] + "/caller-plain.yml", "w").write(
    "name: caller\non:\n  pull_request:\njobs:\n  call:\n    permissions:\n      actions: write\n"
    "    uses: ./.github/workflows/reusable-plain.yml\n"
)
PY
verdict >/dev/null
# shellcheck disable=SC2016
check "the job that inherits the permission is refused as one that is not in the list" 1 "$(message 'job `work`: holds')"

# Widening what the listed job runs.
fresh
edit reusable-cancel-others.yml "s + '      - run: bash scripts/ci/extra.sh\n'"
check "an extra step in the cancel job is refused" 1 "$(verdict)"
check "...by its count" 1 "$(message 'has 3 steps')"

fresh
edit reusable-cancel-others.yml "s.replace('bash \"\$RUNNER_TEMP/others.sh\"', 'bash .github/scripts/ci/cancel-others.sh', 1)"
check "running the working tree's script instead of the base copy is refused" 1 "$(verdict)"
check "...as a step that is not the one listed" 1 "$(message 'is not the step the list holds')"

fresh
edit reusable-cancel-others.yml "s.replace('persist-credentials: false', 'persist-credentials: true', 1)"
check "a checkout that keeps the credentials is refused" 1 "$(verdict)"

fresh
edit reusable-cancel-others.yml "s.replace('actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1', 'actions/checkout@v7', 1)"
check "a checkout that is not pinned by hash is refused" 1 "$(verdict)"
check "...saying so" 1 "$(message 'not pinned by hash')"

fresh
edit reusable-cancel-others.yml "s.replace('    steps:\n', '    env:\n      BASH_ENV: /tmp/x\n    steps:\n', 1)"
check "an environment added to the job is refused" 1 "$(verdict)"
check "...saying the surroundings changed" 1 "$(message 'env')"

fresh
edit reusable-cancel-others.yml "s.replace('GH_TOKEN: \${{ github.token }}', 'GH_TOKEN: \${{ github.token }}\n          LD_PRELOAD: /tmp/x.so', 1)"
check "a variable added to the launcher is refused" 1 "$(verdict)"

fresh
edit audit.yml "s.replace('gh workflow run audit.yml', 'bash scripts/x.sh; gh workflow run audit.yml', 1)"
check "a changed nightly launcher is refused" 1 "$(verdict)"

# A new job that holds the permission and is not listed.
fresh
python3 - "$WORK/tree/.github/workflows" <<'PY'
import sys
d = sys.argv[1]
open(d + "/new.yml", "w").write(
    "name: new\non:\n  pull_request:\njobs:\n  ship:\n    runs-on: ubuntu-latest\n    permissions:\n      actions: write\n"
    "    steps:\n      - run: echo hi\n"
)
PY
check "a new job holding the permission is refused" 1 "$(verdict)"
# shellcheck disable=SC2016
check "...naming the job" 1 "$(message 'job `ship`')"

# Reading permissions that are not write.
fresh
edit ci.yml "s.replace('permissions:\n  contents: read\n', 'permissions:\n  contents: read\n  actions: read\n', 1)"
check "actions: read is not the permission" 0 "$(verdict)"

fresh
check "the job callers of the cancel workflow are accepted with the permission" 0 "$(verdict)"
check "six workflows call the cancel workflow" 6 \
	"$(cat "$WORK"/tree/.github/workflows/*.yml | grep -c 'uses: ./.github/workflows/reusable-cancel-others.yml')"

echo
echo "passed: $pass  failed: $fail"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
