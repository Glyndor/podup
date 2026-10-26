#!/usr/bin/env bash
#
# Behaviour tests for the merge_group trigger check in
# .github/workflows/reusable-workflow-lint.yml: a workflow that emits a
# develop-required check name must trigger on `merge_group` (with
# `checks_requested` in its `types:` where `types:` is set), and no
# other workflow may carry the trigger.
#
# The `protect-develop` ruleset runs a merge queue. In the queue
# GitHub fires `merge_group` on a temporary branch; required checks
# must report on that event or the queue never merges. `paths:` and
# `branches:` filters do not apply, so the trigger has to sit at the
# top level, and a workflow that emits a required check name without
# the trigger leaves the queue waiting for a check that never comes.
#
# The companion rule is the other direction: a workflow that carries
# `merge_group` for any other reason would run inside the queue
# without being asked to. release.yml holds publishing secrets; that
# is the case the second rule exists for, and the script refuses any
# other workflow that adds the trigger.
#
# `reusable-main-guard.yml` is deliberately absent from the required
# list (`develop-only` is required on `main` only and `main` has no
# queue); the script's static map reflects that, and a caller of
# reusable-main-guard.yml without `merge_group` must stay accepted.
#
# A workflow that cannot be parsed is warned and skipped, the same way
# the caller-if and caller-with-defaults steps handle it. A scan that
# finds zero emitting workflows fails closed: an empty answer would be
# a pass that did not look.
#
# Each step's `run:` body is extracted from the workflow and executed
# as it ships, in a temporary tree shaped like a repository. Every
# refusal asserts WHICH message fired. Each refusal is paired with an
# acceptance of the same shape just inside the line so the rule is
# precise.
#
# Requires: python3 with PyYAML (the step under test imports it).
# The fixtures below carry literal `${{ ... }}` expressions: the
# assertion under test reads those characters, so they must reach the
# file unexpanded. Single quotes are the point, not an oversight.
# shellcheck disable=SC2016
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
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

# Pull one step's 'run:' body out of a workflow, dedented, so it can be
# run. Same helper as the other tests in this directory: the point is
# to exercise the script as it ships rather than a copy of it that can
# drift.
step_script() { # $1=workflow path  $2=step name substring
	python3 - "$1" "$2" <<'PY'
import sys
lines = open(sys.argv[1]).read().splitlines()
try:
    start = next(i for i, l in enumerate(lines) if "name: " + sys.argv[2] in l)
except StopIteration:
    sys.exit(0)
try:
    run = next(i for i, l in enumerate(lines) if i > start and l.strip() == "run: |")
except StopIteration:
    sys.exit(0)
body = []
for line in lines[run + 1:]:
    if not line.strip():
        body.append("")
        continue
    if not line.startswith(" " * 10):
        break
    body.append(line[10:])
print("\n".join(body))
PY
}

WORKFLOW="$HERE/.github/workflows/reusable-workflow-lint.yml"
step_script "$WORKFLOW" "A workflow that emits a check required on develop" \
	> "$WORK/merge-group.sh"
check "the merge-group step was extracted from the workflow" "1" \
	"$(grep -c check-merge-group-triggers.py "$WORK/merge-group.sh" | awk '{print ($1>0)}')"

check "the merge-group trigger script lives next to the other GitHub scripts" "1" \
	"$(test -f "$HERE/.github/scripts/checks/check-merge-group-triggers.py" && echo 1 || echo 0)"

# ---------------------------------------------------------------------------
# Fixture helpers
# ---------------------------------------------------------------------------

# A fresh empty tree with '.github/workflows/' and '.github/scripts/'
# (the merge-group trigger check shells out to the script). The
# reusables are not required for the empty-emitter test, but the
# other cases plant callers of reusables in DEVELOP_REQUIRED_REUSABLES,
# so the real reusables travel with the tree.
new() {
	rm -rf "$WORK/t"
	mkdir -p "$WORK/t/.github/workflows" "$WORK/t/.github/scripts/checks"
	for r in \
		reusable-rust-ci.yml \
		reusable-dco.yml \
		reusable-line-limit.yml \
		reusable-main-guard.yml \
		reusable-workflow-lint.yml; do
		cp "$HERE/.github/workflows/$r" "$WORK/t/.github/workflows/"
	done
	cp "$HERE/.github/scripts/checks/check-merge-group-triggers.py" \
		"$WORK/t/.github/scripts/checks/"
	printf '%s' "$WORK/t"
}
run_in() { ( cd "$1" && bash "$2" 2>&1 ); }
said() { printf '%s' "$1" | grep -q -F -- "$2" && echo 1 || echo 0; }

# Write a workflow file. $1=dir $2=filename $3=body (already a
# complete workflow text, no quoting tricks).
write_workflow() {
	printf '%s\n' "$3" > "$1/.github/workflows/$2"
}

# ===========================================================================
# Real-tree positive control: the loaded step body, run against the
# real repository, exits 0 and prints the OK summary once every
# develop-required check has a caller that triggers on `merge_group`.
# ===========================================================================

out="$(cd "$HERE" && bash "$WORK/merge-group.sh")"; rc=$?
check "the real repository tree passes the merge-group trigger assertion" \
	"0" "$rc"
check "and the step prints the OK summary when the real tree is clean" "1" \
	"$(said "$out" 'merge-group-triggers: every develop-required check')"

# ===========================================================================
# A ci.yml-shaped caller of reusable-rust-ci.yml WITHOUT merge_group
# is refused. The reusable emits required checks (Format & lint, Test,
# Coverage, Doc warnings, MSRV, Extra platforms) so the caller is an
# emitter and must carry the trigger.
# ===========================================================================

d="$(new)"
write_workflow "$d" "ci.yml" 'name: CI
on:
  pull_request:
jobs:
  rust:
    uses: ./.github/workflows/reusable-rust-ci.yml
'
out="$(run_in "$d" "$WORK/merge-group.sh")"; rc=$?
check "a ci.yml-shaped caller of reusable-rust-ci without merge_group is refused" \
	"1" "$rc"
check "and the message names ci.yml" "1" "$(said "$out" 'ci.yml')"
check "and the message names the missing trigger" "1" \
	"$(said "$out" 'no `merge_group` trigger')"
check "and the step did not silently pass it" "0" \
	"$(said "$out" 'merge-group-triggers: every develop-required check')"

# ===========================================================================
# Same shape WITH merge_group (types checks_requested) -> accepted.
# ===========================================================================

d="$(new)"
write_workflow "$d" "ci.yml" 'name: CI
on:
  pull_request:
  merge_group:
    types: [checks_requested]
jobs:
  rust:
    uses: ./.github/workflows/reusable-rust-ci.yml
'
out="$(run_in "$d" "$WORK/merge-group.sh")"; rc=$?
check "a ci.yml-shaped caller of reusable-rust-ci WITH merge_group is accepted" \
	"0" "$rc"
check "and the step prints the OK summary" "1" \
	"$(said "$out" 'merge-group-triggers: every develop-required check')"

# ===========================================================================
# merge_group with `types: [destroyed]` only -> refused. The queue
# fires `checks_requested`; a workflow that only listens for other
# merge_group event types will not run there.
# ===========================================================================

d="$(new)"
write_workflow "$d" "ci.yml" 'name: CI
on:
  pull_request:
  merge_group:
    types: [destroyed]
jobs:
  rust:
    uses: ./.github/workflows/reusable-rust-ci.yml
'
out="$(run_in "$d" "$WORK/merge-group.sh")"; rc=$?
check "merge_group with types=[destroyed] is refused" "1" "$rc"
check "and the message names the offending types" "1" \
	"$(said "$out" 'destroyed')"
check "and the message names the required type" "1" \
	"$(said "$out" 'checks_requested')"

# ===========================================================================
# A direct-emitting job named `Supported Podman majors` (DIRECT_EMITTER_NAMES)
# without merge_group -> refused. No reusable call: the required check
# is the job's `name:` itself.
# ===========================================================================

d="$(new)"
write_workflow "$d" "podman-lane.yml" 'name: podman-lane
on:
  pull_request:
jobs:
  podman-majors:
    name: Supported Podman majors
    runs-on: ubuntu-latest
    steps:
      - run: echo green
'
out="$(run_in "$d" "$WORK/merge-group.sh")"; rc=$?
check "a direct emitter (Supported Podman majors) without merge_group is refused" \
	"1" "$rc"
check "and the message names podman-lane.yml" "1" \
	"$(said "$out" 'podman-lane.yml')"
check "and the message names the missing trigger" "1" \
	"$(said "$out" 'no `merge_group` trigger')"

# ===========================================================================
# A release.yml-shaped workflow (no required emitter) with
# merge_group -> refused, names release.yml. The reason: release.yml
# holds publishing secrets, and a workflow that adds the trigger for
# no reason would boot inside the queue.
# ===========================================================================

d="$(new)"
write_workflow "$d" "release.yml" 'name: release
on:
  push:
    tags:
      - "v*"
  workflow_dispatch:
  merge_group:
    types: [checks_requested]
jobs:
  verify:
    runs-on: ubuntu-latest
    steps:
      - run: echo green
'
out="$(run_in "$d" "$WORK/merge-group.sh")"; rc=$?
check "a release.yml-shaped workflow with merge_group is refused" "1" "$rc"
check "and the message names release.yml" "1" \
	"$(said "$out" 'release.yml')"
check "and the message warns about publishing secrets" "1" \
	"$(said "$out" 'publishing secrets')"

# ===========================================================================
# `on:` as a list form `on: [pull_request, merge_group]` on an
# emitter -> accepted. The script must coerce list-form triggers
# into a single-key dict so the rest of the scan iterates uniformly.
# ===========================================================================

d="$(new)"
write_workflow "$d" "ci.yml" 'name: CI
on: [pull_request, merge_group]
jobs:
  rust:
    uses: ./.github/workflows/reusable-rust-ci.yml
'
out="$(run_in "$d" "$WORK/merge-group.sh")"; rc=$?
check "an emitter with on: as a list including merge_group is accepted" \
	"0" "$rc"

# ===========================================================================
# A main-guard.yml-shaped caller of reusable-main-guard.yml without
# merge_group -> accepted. `develop-only` is required on `main` only
# and `main` has no queue, so its caller must NOT be forced to carry
# the trigger.
# ===========================================================================

d="$(new)"
write_workflow "$d" "main-guard.yml" 'name: guard
on:
  pull_request:
    branches: ["main"]
jobs:
  develop-only:
    uses: ./.github/workflows/reusable-main-guard.yml
'
# Without the required-emitting ci.yml, the scan would fail-closed
# on zero emitters. Plant a clean emitter first so the script has
# something to recognise, then verify the main-guard caller is not
# itself flagged.
write_workflow "$d" "ci.yml" 'name: CI
on:
  pull_request:
  merge_group:
    types: [checks_requested]
jobs:
  rust:
    uses: ./.github/workflows/reusable-rust-ci.yml
'
out="$(run_in "$d" "$WORK/merge-group.sh")"; rc=$?
check "main-guard.yml caller of reusable-main-guard without merge_group is accepted" \
	"0" "$rc"
check "and the step did not flag main-guard.yml as needing merge_group" "1" \
	"$(said "$out" 'merge-group-triggers: every develop-required check')"

# ===========================================================================
# A tree with no emitting workflow at all -> refused (scan read
# nothing). The check is over the entire tree, so a "found no
# emitters" answer is itself a defect: it means the ruleset cannot be
# enforced and the script cannot have proved the triggers.
# ===========================================================================

d="$(new)"
write_workflow "$d" "unrelated.yml" 'name: Unrelated
on: pull_request
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo green
'
# Strip the required-emitting reusables and the real ci.yml/dco.yml/
# line-limit.yml/podman-lane.yml/workflow-lint.yml that the new()
# helper copied in, so the only workflow left is one that does not
# emit a required check.
for f in ci.yml dco.yml line-limit.yml podman-lane.yml workflow-lint.yml \
	reusable-rust-ci.yml reusable-dco.yml reusable-line-limit.yml \
	reusable-workflow-lint.yml; do
	rm -f "$d/.github/workflows/$f"
done
out="$(run_in "$d" "$WORK/merge-group.sh")"; rc=$?
check "a tree with no emitting workflow is refused (scan read nothing)" \
	"1" "$rc"
check "and the message says no emitting workflow was found" "1" \
	"$(said "$out" 'no workflow in .github/workflows/ emits a develop-required check')"

# ===========================================================================
# Edge: an unparseable workflow is warned and skipped, the same way
# the caller-if and caller-with-defaults steps handle it. The scan
# still passes if the other workflows are clean, so the fixture
# includes a properly-triggered emitter alongside the broken file.
# ===========================================================================

d="$(new)"
write_workflow "$d" "ci.yml" 'name: CI
on:
  pull_request:
  merge_group:
    types: [checks_requested]
jobs:
  rust:
    uses: ./.github/workflows/reusable-rust-ci.yml
'
printf 'name: [\n' > "$d/.github/workflows/broken.yml"
out="$(run_in "$d" "$WORK/merge-group.sh")"; rc=$?
check "an unparseable workflow does not fail the step" "0" "$rc"
check "but is reported as skipped" "1" "$(said "$out" '::warning')"

echo
echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
