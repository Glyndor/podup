#!/usr/bin/env bash
# Behaviour tests for the Podman lane path classifier and its actual decide step.
set -uo pipefail
export LC_ALL=C
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
script="$HERE/.github/scripts/checks/podman-lane-paths.sh"
workflow="$HERE/.github/workflows/podman-lane.yml"
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
run_case() { # <expected status> <paths>
	local rc=0 out
	out="$(bash "$script" "$2" 2>&1)" || rc=$?
	check "classify $2" "$1" "$rc"
	if [ "$1" = 0 ]; then
		check "exemption explains the result" "nothing in this range can reach the VM" "$out"
	else
		check "refusal names the reason" 1 "$(printf '%s\n' "$out" | grep -cF 'reaches the VM:')"
	fi
}
for path in README.md docs/nested/page.html .github/ISSUE_TEMPLATE/bug.yml LICENSE .editorconfig \
	.github/dependabot.yml .github/CODEOWNERS .github/PULL_REQUEST_TEMPLATE.md \
	fuzz/Cargo.toml fuzz/Cargo.lock fuzz/fuzz_targets/compose.rs \
	bench/timeit/Cargo.toml bench/timeit/Cargo.lock bench/budgets/binary-budget-mib \
	tests/scripts/shell/workflow_behaviour/example.test.sh tests/scripts/powershell/example.Tests.ps1 \
	tests/cli/x.rs tests/compose/parsing/y.rs tests/fixtures/data.yaml tests/hostile/podman/z.rs \
	tests/quadlet/q.rs tests/repository/workflows/w.rs \
	deny.toml about.toml about.hbs debian/control debian/changelog install.sh install.ps1 \
	.github/scripts/release/sign-requirements.txt \
	.github/scripts/release/sign.py .github/scripts/checks/notes.md .github/scripts/checks/rust-ci-inputs.sh \
	.github/scripts/hygiene/check-secrets.sh .github/scripts/hardening/check-hardening.sh \
	.github/scripts/ci/no-deprecation-notices.sh .github/scripts/ci/cancel-others.sh \
	.github/scripts/lib/ci_yaml.py .github/scripts/packaging/stage-payload.py .github/scripts/packaging/build-rpm.sh tests/packaging/verify-rpm.sh \
	.github/actionlint.yaml .github/folder-limit-exceptions.txt .github/vocabulary-exceptions.txt; do
	run_case 0 "$path"
done
# Every existing workflow except the lane has been reviewed as an independent caller.
for file in "$HERE"/.github/workflows/*.yml; do
	path="${file#"$HERE/"}"
	[ "$path" = .github/workflows/podman-lane.yml ] && continue
	run_case 0 "$path"
done
for path in internal/x.rs Cargo.toml Cargo.lock build.rs .cargo/config.toml \
	rust-toolchain rust-toolchain.toml tests/unknown/x.rs tests/engine/x.rs tests/cli.rs \
	tests/engine/engine_integration.rs tests/engine/integration_leak_scan.rs \
	.github/workflows/podman-lane.yml .github/workflows/future.yml \
	.github/podman_lane/known-failures-6 \
	.github/podman_lane/baseline-tests .github/podman_lane/baseline .github/podman_lane/notes.md \
	.github/scripts/checks/podman-lane-paths.sh .github/scripts/checks/is-version-bump.sh \
	.github/scripts/fedora_image/fetch-fedora-image.sh .github/scripts/podman_lane/coverage-floor.sh \
	.github/scripts/unknown/x.sh .github/scripts/top-level.sh .github/scripts/cix/x.sh other.txt \
	.github/scripts/library/x.py .github/scripts/lib.sh .github/scripts/packaging.sh tests/packaging.sh tests/packagingx/y.sh .github/actionlint.yml .github/actionlint.yaml.bak; do
	run_case 1 "$path"
done
run_case 1 $'.github/dependabot.yml\ninternal/x.rs'
run_case 1 $'.github/scripts/hygiene/check-secrets.sh\n.github/scripts/podman_lane/coverage-floor.sh'
run_case 0 $'.github/scripts/hygiene/check-secrets.sh\n.github/scripts/release/sign.py'
run_case 1 $'tests/hostile/podman/z.rs\ntests/engine/engine_integration.rs'
run_case 0 $'tests/hostile/podman/z.rs\ntests/compose/parsing/y.rs\ndocs/commands.md'
run_case 1 $'internal/x.rs\n.github/dependabot.yml'
run_case 0 $'.github/dependabot.yml\nfuzz/Cargo.lock\nbench/timeit/Cargo.lock'
run_case 1 ''
rc=0
out="$(bash "$script" 2>&1)" || rc=$?
check "missing argument refuses" 1 "$rc"
check "missing argument diagnostic" 1 "$(printf '%s\n' "$out" | grep -cF 'usage: podman-lane-paths.sh')"

# Run the YAML's real shell block; fake git supplies names and the reviewed base
# helper, independently of this checkout's script invocation above.
awk '
 /id: files/ { step=1 }
 step && /        run: \|/ { body=1; next }
 body && /^  podman-vm:/ { exit }
 body { sub(/^          /, ""); print }
' "$workflow" >"$WORK/decide.sh"
mkdir "$WORK/bin"
cat >"$WORK/bin/git" <<'GIT'
#!/usr/bin/env bash
case "$1" in
 diff)
  [ "$*" = 'diff --name-only --no-renames base head' ] || exit 2
  [ "${DIFF_ERROR:-0}" = 0 ] || exit 1
  printf '%s\n' "$CHANGED"
  ;;
 show)
  case "$2" in
   base:.github/scripts/checks/is-version-bump.sh) printf 'exit %s\n' "${BUMP_RC:-1}" ;;
   base:.github/scripts/checks/podman-lane-paths.sh)
    [ "${MISSING_HELPER:-0}" = 0 ] || exit 1
    cat "$JUDGE"
    ;;
   *) exit 2 ;;
  esac
  ;;
 *) exit 2 ;;
esac
GIT
chmod +x "$WORK/bin/git"
export JUDGE="$script"
decide_case() { # <event> <paths> <engine>
	local rc=0
	: >"$WORK/output"
	PATH="$WORK/bin:$PATH" EVENT="$1" BASE_REF="${BASE_REF:-develop}" BASE_SHA=base HEAD_SHA=head CHANGED="$2" \
		GITHUB_OUTPUT="$WORK/output" bash "$WORK/decide.sh" >"$WORK/log" 2>&1 || rc=$?
	check "decide $1 $2 exits successfully" 0 "$rc"
	check "decide $1 $2 result" "engine=$3" "$(cat "$WORK/output")"
}
decide_case pull_request .github/dependabot.yml false
decide_case merge_group fuzz/Cargo.lock false
decide_case pull_request $'.github/dependabot.yml\nCargo.toml' true
decide_case merge_group .github/workflows/podman-lane.yml true
decide_case schedule .github/dependabot.yml true
decide_case workflow_dispatch bench/timeit/Cargo.lock true
decide_case pull_request '' true
# A pull request into main boots the VM whatever the range says.
BASE_REF=main decide_case pull_request .github/dependabot.yml true
BASE_REF=main decide_case pull_request README.md true
BASE_REF=Develop decide_case pull_request .github/dependabot.yml true
BASE_REF=feature decide_case pull_request .github/dependabot.yml true
BASE_REF=develop decide_case pull_request .github/dependabot.yml false
DIFF_ERROR=1 decide_case pull_request .github/dependabot.yml true
MISSING_HELPER=1 decide_case pull_request .github/dependabot.yml true
BUMP_RC=0 decide_case pull_request $'Cargo.toml\nCargo.lock\ndebian/changelog' false

# The test folders the classifier exempts must stay out of the two binaries the
# VM runs: a reference from tests/engine to one of them that is not a comment
# would make the exemption unsafe.
check "tests/engine does not use the exempt test folders" 0 \
	"$(grep -rnE 'tests/(cli|compose|fixtures|hostile|quadlet|repository)/|\.\./(cli|compose|fixtures|hostile|quadlet|repository)/' "$HERE/tests/engine" |
		grep -cvE ':[0-9]+:[[:space:]]*//')"

# The exempt-workflow list must be reconsidered if the lane starts calling a
# reusable workflow or a local action. Today its only uses are checkout, and the
# job that cancels the other runs when the lane fails, which runs after the VM.
check "the lane has no local/reusable workflow or action calls beyond the cancel job" 0 \
	"$(grep 'uses:' "$workflow" | grep -cvE 'uses: (actions/checkout@[0-9a-f]{40}|\./\.github/workflows/reusable-cancel-others\.yml)')"
# Only an explicit `false` skips the boot: an output that never arrived boots.
check "Nothing to boot is reached only on an explicit false" 1 \
	"$(grep -A2 -- '- name: Nothing to boot' "$workflow" | grep -cF "if: needs.decide.outputs.engine == 'false'")"
check "no step reads the engine answer as == 'true'" 0 \
	"$(grep -cF "outputs.engine == 'true'" "$workflow")"
check "the required check name remains stable" 1 "$(grep -cF 'name: Supported Podman majors' "$workflow")"
check "the required check waits for the plan and both lanes" 1 "$(grep -cF 'needs: [decide, podman-vm, podman-native]' "$workflow")"
# The lane boots one Podman major, the newest, and the identity gate reads the
# known-failures file of the major the VM actually booted. A gate that finds no
# file skips silently, so every major in the matrix needs its own.
matrix_line="$(grep -E '^[[:space:]]+podman: \[' "$workflow")"
check "the matrix is one line" 1 "$(printf '%s\n' "$matrix_line" | grep -c .)"
majors="$(printf '%s' "$matrix_line" | grep -oE '"[0-9]+"' | tr -d '"')"
check "the matrix runs exactly one Podman major" 1 "$(printf '%s\n' "$majors" | grep -c .)"
for major in $majors; do
	check "Podman $major has its known-failures file" 1 \
		"$([ -f "$HERE/.github/podman_lane/known-failures-$major" ] && echo 1 || echo 0)"
done
check "the lane does not boot a Fedora release image" 0 "$(grep -cE 'releases/[0-9]+/Cloud' "$workflow" "$HERE/.github/scripts/fedora_image/fetch-fedora-image.sh" | awk -F: '{s+=$NF} END {print s+0}')"
# Execute the required check's actual result handler too, for each kind of run.
awk '
 /      - name: Reflect the matrix result/ { step=1 }
 step && /        run: \|/ { body=1; next }
 body && /^  [a-z-]+:/ { exit }
 body { sub(/^          /, ""); print }
' "$workflow" >"$WORK/result.sh"
lane() { # <event> <plan> <engine> <vm> <native> -> exit status
	PLAN="$2" ENGINE="$3" VM="$4" NATIVE="$5" EVENT="$1" bash "$WORK/result.sh" >"$WORK/result.log" 2>&1
	echo $?
}
check "a pull request passes on the native lane" 0 "$(lane pull_request success true skipped success)"
check "the merge queue passes on the native lane" 0 "$(lane merge_group success true skipped success)"
check "a pull request whose plan said nothing to run passes with the native lane skipped" 0 "$(lane pull_request success false skipped skipped)"
check "a native lane skipped without the plan saying so is refused" 1 "$(lane pull_request success true skipped skipped)"
check "...and so is one skipped on an empty plan answer" 1 "$(lane pull_request success '' skipped skipped)"
check "a pull request on which the VM ran is refused" 1 "$(lane pull_request success true success success)"
for result in failure cancelled; do
	check "a native lane that $result is refused" 1 "$(lane pull_request success true skipped $result)"
	check "the schedule's VM that $result is refused" 1 "$(lane schedule success true $result skipped)"
done
check "the schedule passes on the VM" 0 "$(lane schedule success true success skipped)"
check "a dispatch passes on the VM" 0 "$(lane workflow_dispatch success true success skipped)"
check "a schedule on which the native lane ran is refused" 1 "$(lane schedule success true success success)"
check "a schedule whose VM was skipped is refused" 1 "$(lane schedule success true skipped skipped)"
for plan in failure cancelled skipped ""; do
	check "a plan that is '$plan' is refused on a pull request, whatever the lane did" 1 "$(lane pull_request "$plan" true skipped success)"
	check "a plan that is '$plan' is refused on a schedule, whatever the lane did" 1 "$(lane schedule "$plan" true success skipped)"
done
check "the refusal says the plan did not succeed" 1 "$(lane pull_request failure true skipped success >/dev/null; grep -c 'the plan did not succeed' "$WORK/result.log")"
check "the native lane job runs only for a pull request and the queue" 1 \
	"$(awk '/^  podman-native:/{f=1} f&&/github.event_name == .pull_request. \|\| github.event_name == .merge_group./{c++;exit} END{print c+0}' "$workflow")"
check "the VM job never runs for a pull request or the queue" 1 \
	"$(awk '/^  podman-vm:/{f=1} f&&/github.event_name != .pull_request. && github.event_name != .merge_group./{c++;exit} END{print c+0}' "$workflow")"

echo
echo "passed: $pass  failed: $fail"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
