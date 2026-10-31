#!/usr/bin/env bash
# Exercise the judge that lets MSRV and rustdoc skip when nothing they read changed.
set -uo pipefail
export LC_ALL=C
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
script="$HERE/.github/scripts/checks/rust-build-inputs.sh"
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
		check "refusal says a build is required" 1 "$(printf '%s' "$out" | grep -cF 'rust build required:')"
	fi
}
# Skipped: only paths the root build and rustdoc never read.
for path in tests/hostile/input/x.rs docs/commands.md README.md CONTRIBUTING.md \
	bench/run.sh fuzz/fuzz_targets/a.rs debian/rules install.sh install.ps1; do
	run_case 0 "$path"
done
run_case 0 $'tests/a.rs\ndocs/b.md'
# Built: every Rust input, and anything not named, even next to a skipped path.
for path in internal/lib.rs internal/backends/engine/observe/stats/stats_tests.rs \
	Cargo.toml Cargo.lock build.rs .cargo/config.toml rust-toolchain.toml about.hbs \
	.github/workflows/reusable-rust-ci.yml .github/scripts/checks/rust-build-inputs.sh \
	unknown.txt; do
	run_case 1 "$path"
	run_case 1 $'tests/a.rs\n'"$path"
done
run_case 1 ""
check "no argument builds" 1 "$(bash "$script" >/dev/null 2>&1; echo $?)"
# The workflow reads the judge from the base commit and guards the steps with it.
# shellcheck disable=SC2016
check "msrv and doc both read the judge from the base" 2 \
	"$(grep -c 'git show "\$BASE_SHA:.github/scripts/checks/rust-build-inputs.sh"' "$workflow")"
# shellcheck disable=SC2016
check "only pull_request and push can skip" 2 \
	"$(grep -c '\[\[ "\$EVENT" = pull_request || "\$EVENT" = push \]\]' "$workflow")"
check "the build steps are guarded" 5 \
	"$(grep -c "steps.inputs.outputs.build == 'true'" "$workflow")"
echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]
