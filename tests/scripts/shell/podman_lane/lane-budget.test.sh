#!/usr/bin/env bash
#
# The caps of the VM legs of the Podman lane are the measurement and not a number somebody typed.
#
# .github/podman_lane/budget holds, for each leg, the longest the VM took and the margin on it,
# and the caps they give. This test holds that file to its own arithmetic, and the matrix of
# .github/workflows/podman-lane.yml to the file: the cap of qemu, the cap of the job, the job
# taking its cap from the matrix, and qemu taking its from the environment the matrix fills.
# A cap changed in one place and not in the other, a margin below 150 %, a measurement lowered
# to fit a cap that was too small, each turns it red; so do the plants at the end.
#
# Requires: python3 with PyYAML.
# shellcheck disable=SC2016 # the sed expressions are literal workflow text
set -u
export LC_ALL=C

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

# judge <budget file> <workflow file>: one line per problem, none when the caps are the measurement's.
judge() {
	python3 -I - "$1" "$2" "$HERE/.github/scripts/lib" <<'PYEOF'
import math
import sys

sys.path.insert(0, sys.argv[3])
import ci_yaml  # noqa: E402

rows = {}
overhead = None
for line in open(sys.argv[1]):
    line = line.split("#", 1)[0].strip()
    if not line:
        continue
    f = line.split()
    if f[0] == "overhead_minutes":
        overhead = int(f[1])
    else:
        rows[f[0]] = dict(zip(("max", "margin", "vm", "job"), (float(f[1]), int(f[2]), int(f[3]), int(f[4]))))
if set(rows) != {"suite", "coverage"} or overhead is None:
    print("the budget has not exactly the legs suite and coverage and an overhead")
    sys.exit(0)
for leg, r in rows.items():
    minutes = math.ceil(r["max"] * r["margin"] / 100 - 1e-9)
    if r["margin"] < 150:
        print(f"{leg}: a margin of {r['margin']} % is below 150 %")
    if r["vm"] != minutes * 60:
        print(f"{leg}: vm_seconds is {r['vm']}, the measurement times the margin is {minutes * 60}")
    if r["job"] != minutes + overhead:
        print(f"{leg}: job_minutes is {r['job']}, the cap of the VM plus the overhead is {minutes + overhead}")

wf = ci_yaml.load(sys.argv[2])
job = wf["jobs"]["podman-vm"]
if job.get("timeout-minutes") != "${{ matrix.job_minutes }}":
    print("the job does not take its timeout from the matrix")
legs = {i["mode"]: i for i in job["strategy"]["matrix"]["include"]}
if set(legs) != {"suite", "coverage"} or job["strategy"]["matrix"].get("mode") != ["suite", "coverage"]:
    print("the matrix has not exactly the legs suite and coverage")
for leg, r in rows.items():
    got = legs.get(leg, {})
    if got.get("vm_seconds") != r["vm"]:
        print(f"{leg}: the matrix gives qemu {got.get('vm_seconds')} seconds, the budget {r['vm']}")
    if got.get("job_minutes") != r["job"]:
        print(f"{leg}: the matrix gives the job {got.get('job_minutes')} minutes, the budget {r['job']}")
steps = {s.get("name"): s for s in job["steps"]}
boot = steps.get("Boot VM (9p = repo only) + capture", {})
if (boot.get("env") or {}).get("VM_SECONDS") != "${{ matrix.vm_seconds }}" or 'timeout "$VM_SECONDS" qemu' not in boot.get("run", ""):
    print("qemu does not take its cap from the matrix")
if "QEMU_RC=" not in boot.get("run", ""):
    print("the exit status of qemu is not kept, so a cut cannot be told from an end")
verify = steps.get("Verify the VM", {})
if (verify.get("env") or {}).get("VM_SECONDS") != "${{ matrix.vm_seconds }}":
    print("the verification is not told the cap the VM had")
PYEOF
}

BUDGET="$HERE/.github/podman_lane/budget"
WORKFLOW="$HERE/.github/workflows/podman-lane.yml"
check "the caps of the workflow are the measurement times the margin" "" "$(judge "$BUDGET" "$WORKFLOW")"

plant_budget() { # <description> <sed expression> <expected fragment>
	cp "$BUDGET" "$WORK/budget"
	sed -E -i "$2" "$WORK/budget"
	if cmp -s "$WORK/budget" "$BUDGET"; then
		check "the plant changes the budget: $1" changed unchanged
		return
	fi
	check "plant: $1" 1 "$(judge "$WORK/budget" "$WORKFLOW" | grep -cF "$3")"
}
plant_workflow() { # <description> <sed expression> <expected fragment>
	cp "$WORKFLOW" "$WORK/wf.yml"
	sed -E -i "$2" "$WORK/wf.yml"
	if cmp -s "$WORK/wf.yml" "$WORKFLOW"; then
		check "the plant changes the workflow: $1" changed unchanged
		return
	fi
	check "plant: $1" 1 "$(judge "$BUDGET" "$WORK/wf.yml" | grep -cF "$3")"
}
plant_budget "a measurement lowered to fit a cap" 's/^suite( +)49\.4/suite\130.0/' "suite: vm_seconds is 4500"
plant_budget "a cap raised without a measurement" 's/^coverage( +)68\.9( +)150( +)6240/coverage\168.9\2150\39000/' "coverage: vm_seconds is 9000"
plant_budget "a margin below 150 %" 's/^suite( +)49\.4( +)150( +)4500/suite\149.4\2120\33600/' "suite: a margin of 120 % is below 150 %"
plant_budget "a job cap that is not the VM's plus the overhead" 's/^suite( +)49\.4( +)150( +)4500( +)100/suite\149.4\2150\34500\495/' "suite: job_minutes is 95"
plant_budget "a leg that is missing" 's/^coverage /#coverage /' "has not exactly the legs"
plant_workflow "the cap of qemu changed in the workflow only" 's/vm_seconds: 4500/vm_seconds: 4200/' "suite: the matrix gives qemu 4200 seconds"
plant_workflow "the cap of the job changed in the workflow only" 's/job_minutes: 129/job_minutes: 95/' "coverage: the matrix gives the job 95 minutes"
plant_workflow "the job with its own number" 's/timeout-minutes: \$\{\{ matrix.job_minutes \}\}/timeout-minutes: 95/' "does not take its timeout from the matrix"
plant_workflow "qemu with a number of its own" 's/timeout "\$VM_SECONDS" qemu/timeout 4200 qemu/' "qemu does not take its cap from the matrix"
plant_workflow "the exit status of qemu thrown away" 's/QEMU_RC=/QEMU_UNUSED=/' "exit status of qemu is not kept"

echo
echo "passed: $pass  failed: $fail"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
