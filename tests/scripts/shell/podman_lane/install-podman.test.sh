#!/usr/bin/env bash
#
# The native lane's Podman install survives a bad mirror and never lets the
# job go on without Podman 6.2.
#
# On 2026-11-23 `sudo timeout 120 apt-get update` hung on an Ubuntu mirror in
# podup#2632, the next command ran anyway, and the job continued with the
# Podman the image ships (4.9.3) and failed in a step far from the cause. The
# cases below drive .github/scripts/podman_lane/install-podman.sh with stand-ins
# for apt-get, timeout, dpkg, podman and sleep, and read the workflow that calls
# it. Each plant then breaks one rule in a copy and expects the cases to go red.
#
# Requires: python3 with PyYAML.
set -u
export LC_ALL=C

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
REAL_SCRIPT="$HERE/.github/scripts/podman_lane/install-podman.sh"
REAL_WORKFLOW="$HERE/.github/workflows/podman-lane.yml"
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

# Stand-ins. apt-get takes its answers from sequences of exit codes, one per
# call of each kind; an empty sequence means success.
BIN="$WORK/bin"
mkdir -p "$BIN"
cat >"$BIN/apt-get" <<'EOF'
#!/usr/bin/env bash
kind=other
args=" $* "
case "$args" in
*" install "*) kind=install ;;
*" update "*) case "$args" in *Dir::Etc::sourcelist=/dev/null*) kind=scoped ;; *) kind=general ;; esac ;;
esac
echo "$kind $*" >>"$FAKE_LOG"
if [ "$kind" = scoped ]; then
	dir="$(printf '%s\n' "$@" | sed -n 's/^Dir::Etc::sourceparts=//p')"
	echo "parts $(ls "$dir" | tr '\n' ' ')" >>"$FAKE_LOG"
fi
seqfile="$FAKE_DIR/seq-$kind"
n=$(($(cat "$FAKE_DIR/n-$kind" 2>/dev/null || echo 0) + 1))
echo "$n" >"$FAKE_DIR/n-$kind"
var="FAKE_${kind^^}"
read -ra seq <<<"${!var:-}"
exit "${seq[$((n - 1))]:-0}"
EOF
cat >"$BIN/timeout" <<'EOF'
#!/usr/bin/env bash
# timeout -k <grace> <limit> command...
echo "timeout $3 $4 $5" >>"$FAKE_LOG"
shift 3
exec "$@"
EOF
cat >"$BIN/dpkg" <<'EOF'
#!/usr/bin/env bash
echo "dpkg $*" >>"$FAKE_LOG"
EOF
cat >"$BIN/sleep" <<'EOF'
#!/usr/bin/env bash
echo "sleep $*" >>"$FAKE_LOG"
EOF
cat >"$BIN/podman" <<'EOF'
#!/usr/bin/env bash
echo "podman version ${FAKE_PODMAN_VERSION:-6.2.0}"
EOF
chmod +x "$BIN"/*

SOURCE="$WORK/glyndor.sources"
printf 'Types: deb\nURIs: https://apt.glyndor.net\nSuites: stable\nComponents: main\n' >"$SOURCE"

# run <script> [VAR=value ...]: runs the script; sets rc, out, LOG and PHASES_TEXT.
run() {
	local script="$1"
	shift
	FAKE_DIR="$(mktemp -d "$WORK/run.XXXXXX")"
	LOG="$FAKE_DIR/log"
	: >"$LOG"
	: >"$FAKE_DIR/phases"
	rc=0
	out="$(env -i PATH="$BIN:/usr/bin:/bin" FAKE_DIR="$FAKE_DIR" FAKE_LOG="$LOG" PHASES="$FAKE_DIR/phases" \
		PODMAN_FLOOR=6.2.0 INSTALL_PODMAN_SOURCE="$SOURCE" INSTALL_PODMAN_SUDO= "$@" bash "$script" 2>&1)" || rc=$?
	PHASES_TEXT="$(cat "$FAKE_DIR/phases")"
}
count() { grep -c "^$1 " "$LOG" || true; }

# The cases, against the script in $1.
cases() {
	local script="$1"

	run "$script"
	check "a healthy runner installs on the first try" 0 "$rc"
	check "one scoped update, one install" "1 1" "$(count scoped) $(count install)"
	check "no general update when nothing failed" 0 "$(count general)"
	check "the version is recorded" "podman=6.2.0" "$PHASES_TEXT"
	check "the update reads Glyndor's source folder only" "parts glyndor.sources " "$(grep '^parts ' "$LOG" | head -1)"
	check "the update reads no other source list" 1 "$(grep -c 'scoped .*Dir::Etc::sourcelist=/dev/null' "$LOG")"
	check "the lists of the other sources are not dropped" 1 "$(grep -c 'scoped .*APT::Get::List-Cleanup=0' "$LOG")"
	check "the update is bounded" 1 "$(grep -c '^timeout 90 apt-get -q' "$LOG")"
	check "the install is bounded" 1 "$(grep -c '^timeout 240 apt-get -q' "$LOG")"
	check "the install answers its own questions" 1 "$(grep -c '^install .*install -y podman' "$LOG")"

	run "$script" FAKE_SCOPED="124 124"
	check "two stalled updates and then a good one install" 0 "$rc"
	check "three updates, one install" "3 1" "$(count scoped) $(count install)"
	check "the waits grow between tries" "sleep 15 sleep 45" "$(grep '^sleep ' "$LOG" | tr '\n' ' ' | sed 's/ $//')"
	check "an install that never ran does not leave a general update" 0 "$(count general)"
	check "dpkg is repaired between tries" 2 "$(count dpkg)"
	check "the version of the good try is recorded" "podman=6.2.0" "$PHASES_TEXT"

	run "$script" FAKE_SCOPED="124 124 124"
	check "a stalled mirror on every try fails the step" 1 "$rc"
	check "the install never ran on stale lists" 0 "$(count install)"
	check "it stopped after three tries" 3 "$(count scoped)"
	check "the failure says it is the runner's network" 1 "$(printf '%s\n' "$out" | grep -c 'could not be installed after 3 tries')"
	check "the floor is not even read after a failed install" "" "$PHASES_TEXT"

	run "$script" FAKE_INSTALL="100"
	check "an install that fails once is tried again" 0 "$rc"
	check "the general refresh runs after a failed install" 1 "$(count general)"
	check "two installs" 2 "$(count install)"
	check "both updates and the refresh are bounded" 3 "$(grep -c '^timeout 90 apt-get -q' "$LOG")"

	run "$script" FAKE_INSTALL="100 100 100"
	check "an install that fails every time fails the step" 1 "$rc"
	check "three installs, no more" 3 "$(count install)"

	run "$script" INSTALL_PODMAN_TRIES=1 FAKE_SCOPED=124
	check "one try means one try" "1 0 1" "$(count scoped) $(count install) $rc"
	check "no wait after the last try" 0 "$(count sleep)"

	run "$script" FAKE_PODMAN_VERSION=4.9.3
	check "the image's Podman 4.9.3 is refused" 1 "$rc"
	check "the refusal names the floor" 1 "$(printf '%s\n' "$out" | grep -c 'below the 6.2.0 floor')"
	check "a refused version is not recorded" "" "$PHASES_TEXT"
	run "$script" FAKE_PODMAN_VERSION=6.10.1
	check "6.10.1 is above 6.2.0" 0 "$rc"
	run "$script" FAKE_PODMAN_VERSION=6.1.9
	check "6.1.9 is below 6.2.0" 1 "$rc"

	run "$script" PODMAN_FLOOR=
	check "a missing floor is refused" 1 "$rc"
	run "$script" INSTALL_PODMAN_TRIES=0
	check "zero tries is refused" 1 "$rc"
	run "$script" INSTALL_PODMAN_SOURCE="$WORK/none.sources"
	check "a missing source file is refused" 1 "$rc"
	ln -sf "$SOURCE" "$WORK/link.sources"
	run "$script" INSTALL_PODMAN_SOURCE="$WORK/link.sources"
	check "a linked source file is refused" 1 "$rc"
}

# The workflow side: what the job does with the answer. Prints one line per
# problem; empty when the rules hold.
judge() { # <workflow>
	python3 -I - "$1" "$HERE/.github/scripts/lib" <<'EOF'
import sys

sys.path.insert(0, sys.argv[2])
import ci_yaml  # noqa: E402

spec = ci_yaml.load(sys.argv[1])
job = spec["jobs"]["podman-native"]
steps = job["steps"]
names = [s.get("name") for s in steps]
install = next((s for s in steps if s.get("name", "").startswith("Install Podman")), None)
if install is None:
    print("no install step")
    sys.exit(0)
run = install.get("run", "")
if "install-podman.sh" not in run:
    print("the install step does not run the script")
if "apt-get" in run:
    print("the install step calls apt-get itself")
if install.get("id") != "install":
    print("the install step has no id the others can read")
reporting = {"Show what AppArmor refused", "Report the phases"}
position = steps.index(install)
for s in steps[position + 1:]:
    cond = str(s.get("if", ""))
    if s["name"] in reporting:
        if "always()" not in cond:
            print(f"{s['name']}: the report must run when the job failed")
        continue
    if "always()" in cond:
        print(f"{s['name']}: runs after a failed install")
    if s["name"].startswith("Run the Quadlet") and "steps.install.outcome == 'success'" not in cond:
        print("the Quadlet step does not wait for the install")
EOF
}

echo "== the script"
cases "$REAL_SCRIPT"

echo "== the workflow"
check "the real workflow follows the rules" "" "$(judge "$REAL_WORKFLOW")"

plant_wf() { # <description> <expected output fragment> <sed expression>
	local copy="$WORK/wf-$RANDOM.yml"
	sed -E "$3" "$REAL_WORKFLOW" >"$copy"
	if cmp -s "$copy" "$REAL_WORKFLOW"; then
		check "plant changes the workflow: $1" changed unchanged
		return
	fi
	check "plant: $1" 1 "$(judge "$copy" | grep -c "$2")"
}
plant_wf "the Quadlet tests run after any outcome again" "runs after a failed install" \
	"s/if: \\$\\{\\{ !cancelled\\(\\) && steps.install.outcome == 'success' \\}\\}/if: always()/"
plant_wf "the Quadlet tests no longer wait for the install" "does not wait for the install" \
	"s/steps.install.outcome == 'success'/steps.install.outcome != 'skipped'/"
plant_wf "the install step without an id" "no id the others can read" \
	"s/^        id: install\$/        # no id/"
plant_wf "the report no longer runs after a failure" "must run when the job failed" \
	"/name: Report the phases/{n;s/if: always\\(\\)/if: success()/}"
plant_wf "the install step back to calling apt-get" "calls apt-get itself" \
	"s#bash .github/scripts/podman_lane/install-podman.sh#sudo timeout 120 apt-get update#"
plant_wf "the install step no longer runs the script" "does not run the script" \
	"s#bash .github/scripts/podman_lane/install-podman.sh#true#"

echo "== the plants against the script"
plant_script() { # <description> <sed expression>
	local copy="$WORK/script-$RANDOM.sh" before="$fail"
	sed -E "$2" "$REAL_SCRIPT" >"$copy"
	if cmp -s "$copy" "$REAL_SCRIPT"; then
		check "plant changes the script: $1" changed unchanged
		return
	fi
	local saved_pass="$pass"
	cases "$copy" >/dev/null
	local caught=$((fail - before))
	# The cases ran against a broken copy: their failures are the point, not failures of this test.
	fail="$before"
	pass="$saved_pass"
	check "plant: $1" 1 "$([ "$caught" -gt 0 ] && echo 1 || echo 0)"
}
plant_script "the update asked of every source again" 's#-o Dir::Etc::sourcelist=/dev/null ##'
plant_script "the update dropping the cached lists" 's# -o APT::Get::List-Cleanup=0##'
# shellcheck disable=SC2016 # the expressions are literal script text
plant_script "the update ignored when it fails" 's#if apt 90 "\$\{only_glyndor\[@\]\}" update; then#if apt 90 "${only_glyndor[@]}" update || true; then#'
plant_script "a single try" 's#^tries="\$\{INSTALL_PODMAN_TRIES:-3\}"#tries=1#'
# shellcheck disable=SC2016 # the expressions are literal script text
plant_script "no wait between tries" 's#^\t\tsleep "\$\{waits\[\$idx\]\}"#\t\ttrue#'
plant_script "the floor not read" 's#^if \[ "\$\(printf .*#if false; then#'
plant_script "a failed install that passes" 's#fail "Podman could not be installed after#echo "Podman could not be installed after#'
# shellcheck disable=SC2016 # the expressions are literal script text
plant_script "the install not bounded" 's#apt 240 install#"${priv[@]}" apt-get -q install#'

echo
echo "passed: $pass  failed: $fail"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
