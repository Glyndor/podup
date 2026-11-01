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
	.github/scripts/release/sign-requirements.txt; do
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
	.github/podman_lane/known-failures-5 .github/podman_lane/known-failures-6 \
	.github/podman_lane/baseline-tests .github/podman_lane/baseline .github/podman_lane/notes.md \
	.github/scripts/checks/podman-lane-paths.sh .github/scripts/checks/is-version-bump.sh \
	.github/scripts/fedora_image/fetch-fedora-image.sh .github/scripts/release/sign.py \
	.github/scripts/checks/notes.md other.txt; do
	run_case 1 "$path"
done
run_case 1 $'.github/dependabot.yml\ninternal/x.rs'
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
	PATH="$WORK/bin:$PATH" EVENT="$1" BASE_SHA=base HEAD_SHA=head CHANGED="$2" \
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
# reusable workflow or a local action. Today its only uses are checkout.
check "the lane has no local/reusable workflow or action calls" 0 \
	"$(grep 'uses:' "$workflow" | grep -cvE 'uses: actions/checkout@[0-9a-f]{40}')"
check "Nothing to boot keeps the non-engine path successful" 1 \
	"$(grep -A2 -- '- name: Nothing to boot' "$workflow" | grep -cF "if: needs.decide.outputs.engine != 'true'")"
check "the required check name remains stable" 1 "$(grep -cF 'name: Supported Podman majors' "$workflow")"
check "the required check waits for the matrix" 1 "$(grep -cF 'needs: podman-vm' "$workflow")"
# Execute the required check's actual result handler too.
awk '
 /      - name: Reflect the matrix result/ { step=1 }
 step && /        run: \|/ { body=1; next }
 body { sub(/^          /, ""); print }
' "$workflow" >"$WORK/result.sh"
for result in success failure cancelled skipped; do
	rc=0
	RESULT="$result" bash "$WORK/result.sh" >"$WORK/result.log" 2>&1 || rc=$?
	if [ "$result" = success ]; then
		check "required check accepts a successful no-boot matrix" 0 "$rc"
	else
		check "required check refuses $result" 1 "$rc"
		check "required check explains $result" 1 "$(grep -cF "result: $result" "$WORK/result.log")"
	fi
done

echo
echo "passed: $pass  failed: $fail"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
