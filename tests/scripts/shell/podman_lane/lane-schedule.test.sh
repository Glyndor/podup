#!/usr/bin/env bash
#
# The verdict of the Podman lane for every kind of run, and the slots of its schedule.
#
# The step `Reflect the matrix result` of `podman-majors` is run as it is written in the workflow,
# with the result of each job it reads set by the case. It is the one answer a pull request and the
# merge queue are held to, and on the schedule it answers for three slots a day:
#
#   - the first slot boots the VM and runs the native lane unless a run excused it;
#   - the other slots run the native lane only when `slot` did not excuse them, and never the VM;
#   - a dispatch boots the VM and never the native lane;
#   - a pull request and the merge queue never boot the VM and never ask the slot.
#
# Each rule that can be taken out of the step is taken out in a copy and the case that needs it must
# then read the other way. The schedule itself is read from the workflow: three slots, the first one
# being the string that `podman-vm` and the verdict compare with.
#
# Requires: python3 with PyYAML.
# shellcheck disable=SC2016 # the plants hold literal script text
set -u
export LC_ALL=C

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
WF="$HERE/.github/workflows/podman-lane.yml"
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

step() { # <workflow file> -> the script of the verdict
	python3 -I - "$1" << 'PY'
import sys, yaml
spec = yaml.safe_load(open(sys.argv[1]))
for s in spec["jobs"]["podman-majors"]["steps"]:
    if s.get("name") == "Reflect the matrix result":
        print(s["run"])
PY
}
step "$WF" > "$WORK/verdict.sh"
[ -s "$WORK/verdict.sh" ] || { echo "FAIL  the step of the verdict was not found"; exit 1; }

FIRST="0 9 * * *"
BACKUP="20 13 * * *"
verdict() { # <script> <event> <schedule> <plan> <engine> <slot> <due> <vm> <native> -> pass | fail
	if env EVENT="$2" SCHEDULE="$3" PLAN="$4" ENGINE="$5" SLOT="$6" DUE="$7" VM="$8" NATIVE="$9" bash "$1" > /dev/null 2>&1; then echo pass; else echo fail; fi
}
# <description> <expected> <event> <schedule> <plan> <engine> <slot> <due> <vm> <native>
case_() {
	check "$1" "$2" "$(verdict "$WORK/verdict.sh" "$3" "$4" "$5" "$6" "$7" "$8" "$9" "${10}")"
}

# A pull request and the merge queue.
case_ "pull request: native passed" pass pull_request "" success true skipped "" skipped success
case_ "merge queue: native passed" pass merge_group "" success true skipped "" skipped success
case_ "pull request: nothing can reach the engine" pass pull_request "" success false skipped "" skipped skipped
case_ "pull request: native skipped without the plan saying so" fail pull_request "" success true skipped "" skipped skipped
case_ "pull request: native failed" fail pull_request "" success true skipped "" skipped failure
case_ "pull request: the VM ran" fail pull_request "" success true skipped "" success success
case_ "pull request: the slot was asked" fail pull_request "" success true success true skipped success
case_ "pull request: the plan failed" fail pull_request "" failure "" skipped "" skipped success

# The first slot of the schedule.
case_ "first slot: VM and native passed" pass schedule "$FIRST" success true success true success success
case_ "first slot: VM passed, native excused" pass schedule "$FIRST" success true success false success skipped
case_ "first slot: native skipped though due" fail schedule "$FIRST" success true success true success skipped
case_ "first slot: native skipped and the slot said nothing" fail schedule "$FIRST" success true success "" success skipped
case_ "first slot: VM failed" fail schedule "$FIRST" success true success true failure success
case_ "first slot: VM skipped" fail schedule "$FIRST" success true success true skipped success
case_ "first slot: native failed" fail schedule "$FIRST" success true success true success failure
case_ "first slot: the slot job failed" fail schedule "$FIRST" success true failure "" success success
case_ "first slot: the slot job was skipped" fail schedule "$FIRST" success true skipped "" success success

# The other slots.
case_ "backup slot: native passed, VM skipped" pass schedule "$BACKUP" success true success true skipped success
case_ "backup slot: excused" pass schedule "$BACKUP" success true success false skipped skipped
case_ "backup slot: the VM ran" fail schedule "$BACKUP" success true success true success success
case_ "backup slot: native failed" fail schedule "$BACKUP" success true success true skipped failure
case_ "backup slot: native skipped though due" fail schedule "$BACKUP" success true success true skipped skipped

# A dispatch.
case_ "dispatch: VM passed, native skipped" pass workflow_dispatch "" success true skipped "" success skipped
case_ "dispatch: VM failed" fail workflow_dispatch "" success true skipped "" failure skipped
case_ "dispatch: native ran" fail workflow_dispatch "" success true skipped "" success success

# The rules, one at a time.
plant() { # <description> <sed expression> <event> <schedule> <slot> <due> <vm> <native> <what the plant reads>
	local copy="$WORK/plant.sh"
	sed -E "$2" "$WORK/verdict.sh" > "$copy"
	if cmp -s "$copy" "$WORK/verdict.sh"; then
		check "$1: the plant changes the step" changed same
		return
	fi
	check "$1: the real step refuses it" fail "$(verdict "$WORK/verdict.sh" "$3" "$4" success true "$5" "$6" "$7" "$8")"
	check "$1: without the rule it passes" "$9" "$(verdict "$copy" "$3" "$4" success true "$5" "$6" "$7" "$8")"
}
plant "the slot not asked" 's/\[ "\$SLOT" = success \] \|\| fail/true || fail/' schedule "$FIRST" failure "" success success pass
plant "the due flag not read" 's/skipped\) \[ "\$DUE" = false \] \|\| fail/skipped) true || fail/' schedule "$FIRST" success true success skipped pass
plant "the VM allowed on a backup slot" 's/\[ "\$VM" = skipped \] \|\| fail "the VM ran on a slot/true || fail "the VM ran on a slot/' schedule "$BACKUP" success true success success pass
plant "the VM not required on the first slot" 's/\[ "\$VM" = success \] \|\| fail "the VM did not pass \(result: \$VM\)"$/true/' schedule "$FIRST" success true failure success pass
plant "the first slot read as any slot" 's/"\$SCHEDULE" = "0 9 \* \* \*"/-n "$SCHEDULE"/' schedule "$BACKUP" success true success success pass

# The schedule is the three slots, the first being what the job `podman-vm` and the verdict compare with.
slots="$(python3 -I - "$WF" << 'PY'
import sys, yaml
spec = yaml.safe_load(open(sys.argv[1]))
on = spec.get("on", spec.get(True))
print("|".join(c["cron"] for c in on["schedule"]))
PY
)"
check "the schedule is the first slot and two backups" "$FIRST|$BACKUP|20 17 * * *" "$slots"
vm_if="$(python3 -I - "$WF" << 'PY'
import sys, yaml
spec = yaml.safe_load(open(sys.argv[1]))
print(spec["jobs"]["podman-vm"]["if"])
PY
)"
case "$vm_if" in *"github.event.schedule == '$FIRST'"*) first=yes ;; *) first=no ;; esac
check "the VM compares the schedule with the first slot" yes "$first"
case "$(grep -c "$FIRST" "$WF")" in 0) c=none ;; *) c=some ;; esac
check "the first slot is written in the workflow" some "$c"

echo
echo "passed: $pass  failed: $fail"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
