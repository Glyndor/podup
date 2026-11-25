#!/usr/bin/env bash
#
# The VM of the Podman lane cannot rewrite what judges it.
#
# The VM shares the workspace of the runner, and the verification runs on the runner after the VM
# has had its say. A guest that could write the script that verifies it, or the list of failures
# it is allowed, could declare itself green. So:
#
#   - qemu shares the workspace read-only (`readonly=on` on the -virtfs line), and the guest mounts
#     it `ro` (the seed's template); it only copies it;
#   - what judges the VM is a copy made BEFORE it boots, in $RUNNER_TEMP/judge, outside the
#     workspace, and the step "Verify the VM" runs that copy, with its paths, and not the workspace;
#   - that copy holds everything the judge reads: the script, the floor of the coverage, the list of
#     known failures and the baseline. A copy that lacks one makes the judge fail, not read the
#     workspace, which is shown here by running the judge from a copy on a log made for it.
#
# Each plant removes one piece in a copy of the workflow or of the template and must turn a case red.
#
# Requires: python3 with PyYAML.
# shellcheck disable=SC2016,SC1003 # the plants hold literal workflow text
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

# judge <workflow> <template>: one line per problem; none when the VM cannot reach its judge.
judge() {
	python3 -I - "$1" "$2" "$HERE/.github/scripts/lib" <<'PYEOF'
import re
import sys

sys.path.insert(0, sys.argv[3])
import ci_yaml  # noqa: E402

wf = ci_yaml.load(sys.argv[1])
steps = wf["jobs"]["podman-vm"]["steps"]
names = [s.get("name") for s in steps]
boot = steps[names.index("Boot VM (9p = repo only) + capture")]
virtfs = [l for l in boot["run"].splitlines() if "-virtfs" in l]
if len(virtfs) != 1:
    print("the VM is not given exactly one shared folder")
elif "readonly=on" not in re.split(r"[, ]", virtfs[0]):
    print("the shared folder is not read-only for qemu (readonly=on)")
elif "path=${{ github.workspace }}" not in virtfs[0]:
    print("the shared folder is not the workspace, which this test knows how to hold")
tpl = open(sys.argv[2]).read()
mounts = [l for l in tpl.splitlines() if "mount -t 9p" in l]
if len(mounts) != 1:
    print("the seed does not mount the shared folder exactly once")
elif not re.search(r"mount -t 9p -o (?:\S*,)?ro(?:,\S*)? ", mounts[0]):
    print("the guest does not mount the shared folder read-only (-o ro)")
if "Copy the judge out of the workspace" not in names:
    print("the judge is not copied out of the workspace")
else:
    copy = steps[names.index("Copy the judge out of the workspace")]
    if names.index("Copy the judge out of the workspace") > names.index("Boot VM (9p = repo only) + capture"):
        print("the judge is copied after the VM has booted")
    run = copy.get("run", "")
    for needle in ('judge="$RUNNER_TEMP/judge"', "cp -R .github/scripts/podman_lane", "cp -R .github/podman_lane"):
        if needle not in run:
            print(f"the copy of the judge lacks {needle}")
verify = steps[names.index("Verify the VM")]
vrun = verify["run"]
if 'judge="$RUNNER_TEMP/judge"' not in vrun or 'GITHUB_WORKSPACE="$judge" bash "$judge/.github/scripts/podman_lane/verify-vm-log.sh"' not in vrun:
    print("the verification does not run the copy of the judge with its own paths")
if re.search(r'bash "\$GITHUB_WORKSPACE/\.github/scripts/podman_lane/verify-vm-log\.sh"', vrun):
    print("the verification runs the script of the workspace, which the VM could have written")
PYEOF
}

WF="$HERE/.github/workflows/podman-lane.yml"
TPL="$HERE/.github/scripts/podman_lane/user-data.tpl"
check "the VM cannot reach its judge" "" "$(judge "$WF" "$TPL")"

# The copy is enough for the judge: run it from a copy made the way the workflow makes it, on a log
# of a suite that finished, with a workspace that does not exist.
mkdir -p "$WORK/judge/.github/scripts" "$WORK/t"
cp -R "$HERE/.github/scripts/podman_lane" "$WORK/judge/.github/scripts/podman_lane"
cp -R "$HERE/.github/podman_lane" "$WORK/judge/.github/podman_lane"
{
	printf 'PODMAN_BEGIN version=podman version 6.2.0  RUST=rustc PODMAN_END\n'
	printf 'LOGDRIVER_BEGIN value=k8s-file LOGDRIVER_END\n'
	printf 'PODUP_TESTS_RC_BEGIN rc=0 PODUP_TESTS_RC_END\n'
	printf 'PODUP_LEAK_SCAN_BEGIN rc=0 PODUP_LEAK_SCAN_END\n'
	printf 'PODUP_QUADLET_GENERATOR_BEGIN rc=0 PODUP_QUADLET_GENERATOR_END\n'
	printf 'PODUP_SUMMARY_BEGIN pass=301 fail=0 flaky=0 PODUP_SUMMARY_END\n'
	printf 'PODUP_FAILED_TESTS_BEGIN tests= PODUP_FAILED_TESTS_END\n'
	printf 'PROBE_DONE\n'
} >"$WORK/t/vm.log"
run_judge() { # <judge dir> <mode>
	: >"$WORK/t/summary.md"
	rc=0
	RUNNER_TEMP="$WORK/t" GITHUB_WORKSPACE="$1" GITHUB_STEP_SUMMARY="$WORK/t/summary.md" EVENT=schedule QEMU_RC=0 VM_SECONDS=4500 \
		bash "$1/.github/scripts/podman_lane/verify-vm-log.sh" "$2" >/dev/null 2>&1 || rc=$?
}
run_judge "$WORK/judge" suite
check "the copy of the judge holds everything it reads: a finished suite is green from it" "0" "$rc"
for missing in .github/podman_lane/baseline-tests .github/podman_lane/known-failures-6; do
	cp -R "$WORK/judge" "$WORK/judge-less"
	rm -f "$WORK/judge-less/$missing"
	run_judge "$WORK/judge-less" suite
	check "a copy without $(basename "$missing") makes the judge fail, not read the workspace" "1" "$([ "$rc" -ne 0 ] && echo 1 || echo 0)"
	rm -rf "$WORK/judge-less"
done
{ sed -n '1,2p' "$WORK/t/vm.log"; printf 'PODUP_COVERAGE=90.67%%\nPROBE_DONE\n'; } >"$WORK/t/vm-cov.log"
cp "$WORK/t/vm.log" "$WORK/t/vm-suite.log"
cp "$WORK/t/vm-cov.log" "$WORK/t/vm.log"
run_judge "$WORK/judge" coverage
check "a finished coverage run is green from the copy" "0" "$rc"
cp -R "$WORK/judge" "$WORK/judge-less"
rm -f "$WORK/judge-less/.github/scripts/podman_lane/coverage-floor.sh"
run_judge "$WORK/judge-less" coverage
check "a copy without the floor of the coverage makes the judge fail, not read the workspace" "1" "$([ "$rc" -ne 0 ] && echo 1 || echo 0)"
rm -rf "$WORK/judge-less"

echo "== the plants"
plant() { # <description> <file: wf|tpl> <old text, once> <new text>
	local wf="$WORK/wf.yml" tpl="$WORK/tpl"
	cp "$WF" "$wf"
	cp "$TPL" "$tpl"
	local target="$wf"
	[ "$2" = tpl ] && target="$tpl"
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
	check "plant: $1" 1 "$([ -n "$(judge "$wf" "$tpl")" ] && echo 1 || echo 0)"
}
plant "the shared folder writable for qemu" wf ',id=repo,readonly=on \' ',id=repo \'
plant "the guest mounting the folder read-write" tpl 'mount -t 9p -o ro,trans=virtio' 'mount -t 9p -o trans=virtio'
plant "the judge not copied" wf '      - name: Copy the judge out of the workspace
' '      - name: Copy nothing
'
plant "the judge copied without the lists it reads" wf '          cp -R .github/podman_lane "$judge/.github/podman_lane"
' ''
plant "the verification running the script of the workspace" wf 'GITHUB_WORKSPACE="$judge" bash "$judge/.github/scripts/podman_lane/verify-vm-log.sh" "$MODE"' 'bash "$GITHUB_WORKSPACE/.github/scripts/podman_lane/verify-vm-log.sh" "$MODE"'
plant "the verification running the copy but reading the workspace's lists" wf 'GITHUB_WORKSPACE="$judge" bash' 'bash'

echo
echo "passed: $pass  failed: $fail"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
