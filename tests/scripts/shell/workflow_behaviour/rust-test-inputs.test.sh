#!/usr/bin/env bash
# Exercise the judge that lets Test, Test (<os>) and Coverage skip when nothing
# the root tests read changed.
set -uo pipefail
export LC_ALL=C
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
script="$HERE/.github/scripts/checks/rust-test-inputs.sh"
workflow="$HERE/.github/workflows/reusable-rust-ci.yml"
pass=0
fail=0
check() {
	if [ "$2" = "$3" ]; then
		echo "ok    $1"
		pass=$((pass + 1))
	else
		echo "FAIL  $1 (expected $2, got $3)"
		fail=$((fail + 1))
	fi
}
run_case() {
	local rc=0 out
	out="$(bash "$script" "$2" 2>&1)" || rc=$?
	check "classify $(printf '%s' "$2" | tr '\n' ' ')" "$1" "$rc"
	if [ "$1" = 1 ]; then
		check "refusal says tests are required" 1 "$(printf '%s' "$out" | grep -cF 'rust tests required:')"
	fi
}
# The classification table, run against $script. Kept in a function so the
# sabotage below can run the very same table against a broken judge.
table() {
	# Skipped: only paths no test opens.
	for path in LICENSE bench/run.sh bench/scenarios/a.sh .github/CODEOWNERS \
		.github/ISSUE_TEMPLATE/bug.yml .github/PULL_REQUEST_TEMPLATE.md; do
		run_case 0 "$path"
	done
	run_case 0 $'LICENSE\nbench/run.sh'
	# Tested: the integration tests themselves, every file they open, and
	# anything not named, even next to a skipped path.
	for path in tests/hostile/input/x.rs tests/fixtures/a.yml tests/scripts/shell/a.sh \
		docs/commands.md docs/assets/podup-demo.gif README.md CONTRIBUTING.md \
		install.sh install.ps1 debian/control debian/rules fuzz/Cargo.toml \
		fuzz/fuzz_targets/a.rs bench/timeit/Cargo.toml .github/dependabot.yml \
		.github/workflows/ci.yml .github/workflows/audit.yml \
		.github/workflows/reusable-rust-ci.yml .github/scripts/checks/rust-test-inputs.sh \
		.github/scripts/checks/check-branch-conclusion.sh \
		internal/lib.rs Cargo.toml Cargo.lock build.rs .cargo/config.toml deny.toml \
		about.hbs unknown.txt; do
		run_case 1 "$path"
		run_case 1 $'LICENSE\n'"$path"
	done
	run_case 1 ""
	check "no argument tests" 1 "$(bash "$script" >/dev/null 2>&1; echo $?)"
}
table
# Sabotage: a judge that exempts tests/* (the mistake the build judge may make)
# must fail this table. If the copy is not actually broken, or the table still
# passes, the table proves nothing.
broken="$(mktemp)"
trap 'rm -f "$broken"' EXIT
sed 's#bench/\* | LICENSE#tests/* | bench/* | LICENSE#' "$script" >"$broken"
check "sabotage changes the judge" 1 "$(cmp -s "$script" "$broken"; echo $?)"
real_pass=$pass real_fail=$fail
script="$broken"
table >/dev/null
sabotaged_fail=$((fail - real_fail))
pass=$real_pass fail=$real_fail
check "a judge that exempts tests/* fails the table" 1 "$([ "$sabotaged_fail" -gt 0 ] && echo 1 || echo 0)"
script="$HERE/.github/scripts/checks/rust-test-inputs.sh"
# The workflow reads the judge from the base commit and guards the steps with it.
# shellcheck disable=SC2016
check "test, test-extra and coverage read the judge from the base" 3 \
	"$(grep -c 'git show "\$BASE_SHA:.github/scripts/checks/rust-test-inputs.sh"' "$workflow")"
# shellcheck disable=SC2016
check "only pull_request, push and merge_group can skip" 5 \
	"$(grep -c '\[\[ "\$EVENT" = pull_request || "\$EVENT" = push || "\$EVENT" = merge_group \]\]' "$workflow")"
check "three jobs decide with the test judge" 3 \
	"$(grep -c 'Decide whether the root test inputs changed' "$workflow")"
# 6 steps behind the build judge (msrv, doc) plus 11 behind this one: test 3,
# test-extra 3, coverage 5 (toolchain, cache, llvm-cov, Podman socket, gate).
check "every expensive step is guarded" 17 \
	"$(grep -c "steps.inputs.outputs.build == 'true'" "$workflow")"
echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
