#!/usr/bin/env bash
# Exercise the judge that lets the Debian package skip when nothing it reads
# changed.
#
# The judge is `.github/scripts/checks/debian-ci-inputs.sh`, read by the
# workflow from the reviewed base commit. It answers one question: does this
# range touch anything the package build reads? Exit 0 means the package may
# be skipped, exit 1 means it must be built.
#
# Every rule in the judge has a case below, and every case names the rule it
# expects, not only the verdict. Removing a path from the judge's list of
# paths that force the package to be built changes the reason the judge
# gives, and this file fails: a rule nobody exercises is a rule that can be
# deleted without anybody noticing, which is how a lane stops running without
# a decision.
#
# The last cases go further and take the rule away for real: they run a copy
# of the judge with one rule deleted and require the answer to change. A rule
# whose removal changes nothing was not what decided the verdict.
set -uo pipefail
export LC_ALL=C
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
script="$HERE/.github/scripts/checks/debian-ci-inputs.sh"
workflow="$HERE/.github/workflows/reusable-rust-debian.yml"
caller="$HERE/.github/workflows/debian-build.yml"
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

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
git init -q -b main "$work"
git -C "$work" config user.email classifier@example.invalid
git -C "$work" config user.name classifier
printf 'base\n' >"$work/README.md"
git -C "$work" add -A
git -C "$work" commit -q -m base
base="$(git -C "$work" rev-parse HEAD)"

# change <path>...: commit the paths on top of the base, print the head.
change() { # <path>...
	local path
	git -C "$work" reset -q --hard "$base"
	git -C "$work" clean -qfdx
	for path in "$@"; do
		mkdir -p "$work/$(dirname "$path")"
		printf 'changed\n' >"$work/$path"
	done
	git -C "$work" add -A
	git -C "$work" commit -q -m change
	git -C "$work" rev-parse HEAD
}

# judge <event> <base> <head> [judge]: the verdict, the reason and the exit
# code, one per line. The judge defaults to the one in the tree.
judge() { # <event> <base> <head> [judge]
	local out="$work/output" reason rc=0 judge_script="${4:-$script}"
	: >"$out"
	reason="$(
		cd "$work" || exit 1
		GITHUB_OUTPUT="$out" EVENT="$1" bash "$judge_script" "$2" "$3"
	)" || rc=$?
	printf '%s\n%s\n%s\n' "$(sed -n 's/^run=//p' "$out")" "$reason" "$rc"
}

# must_run <reason> <path>...: the package is built, and this is why.
must_run() { # <reason> <path>...
	local reason="$1" path head out
	shift
	for path in "$@"; do
		head="$(change "$path")"
		out="$(judge pull_request "$base" "$head")"
		check "classify $path builds the package" true "$(printf '%s\n' "$out" | sed -n 1p)"
		check "classify $path exits 1" 1 "$(printf '%s\n' "$out" | sed -n 3p)"
		check "classify $path names its rule" 1 \
			"$(printf '%s\n' "$out" | sed -n 2p | grep -cF "$reason")"
	done
}

# may_skip <path>...: the package may be skipped, and the judge says so.
may_skip() { # <path>...
	local path head out
	for path in "$@"; do
		head="$(change "$path")"
		out="$(judge pull_request "$base" "$head")"
		check "classify $path skips the package" false "$(printf '%s\n' "$out" | sed -n 1p)"
		check "classify $path exits 0" 0 "$(printf '%s\n' "$out" | sed -n 3p)"
		check "classify $path says nothing reaches it" 1 \
			"$(printf '%s\n' "$out" | sed -n 2p | grep -cF 'nothing in this range can reach the Debian package')"
	done
}

# mixed <reason> <path>...: several paths changed at once, one of which may
# be skipped and one of which may not.
mixed() { # <reason> <path>...
	local reason="$1" head out
	shift
	head="$(change "$@")"
	out="$(judge pull_request "$base" "$head")"
	check "classify $* builds the package" true "$(printf '%s\n' "$out" | sed -n 1p)"
	check "classify $* exits 1" 1 "$(printf '%s\n' "$out" | sed -n 3p)"
	check "classify $* names its rule" 1 \
		"$(printf '%s\n' "$out" | sed -n 2p | grep -cF "$reason")"
}

# Every path the package build reads, and every path whose effect on it
# nobody measured.
must_run "changes the classifier" .github/scripts/checks/debian-ci-inputs.sh
must_run "changes the dependency graph" Cargo.toml Cargo.lock bench/timeit/Cargo.lock
must_run "changes the crate" internal/lib.rs internal/backends/engine/observe/stats/stats.rs
must_run "changes the build script" build.rs
must_run "changes the packaging" debian/rules debian/control
must_run "changes the Rust test suite" \
	tests/hostile/input/parsing/hostile_compose.rs tests/fixtures/interpolation/ordinary-config.yml \
	tests/scripts/stray.rs tests/scripts/shell/notes.md
must_run "changes the CI" .github/workflows/ci.yml .github/workflows/reusable-rust-debian.yml
must_run "changes the checks CI runs" .github/scripts/checks/check-planning-words.sh
must_run "changes an installer" install.sh install.ps1
must_run "is not a script-only test change" unknown.txt README.md docs/commands.md

# One path that may be skipped does not excuse one that may not.
mixed "changes the crate" tests/scripts/shell/workflow_behaviour/debian-ci-inputs.test.sh internal/lib.rs
mixed "changes the CI" tests/scripts/powershell/install-python-isolation.Tests.ps1 .github/workflows/ci.yml

# Skip: the shell and PowerShell test suites, which cargo compiles no part of.
may_skip tests/scripts/shell/workflow_behaviour/debian-ci-inputs.test.sh \
	tests/scripts/shell/workflow_behaviour/rust-ci-inputs.test.sh \
	tests/scripts/powershell/install-python-isolation.Tests.ps1 \
	tests/scripts/shell/notes.sh
change tests/scripts/shell/notes.sh tests/scripts/powershell/install-python-isolation.Tests.ps1
out="$(judge pull_request "$base" "$(git -C "$work" rev-parse HEAD)")"
check "several skippable paths skip the package" false "$(printf '%s\n' "$out" | sed -n 1p)"

# Fail closed: no range, no history, an event that cannot skip, an empty
# range. Every one of them builds the package.
check "no range builds the package" true "$(judge pull_request '' '' | sed -n 1p)"
check "an unknown base builds the package" true "$(judge pull_request deadbeef "$base" | sed -n 1p)"
check "an unknown head builds the package" true "$(judge pull_request "$base" deadbeef | sed -n 1p)"
check "a schedule builds the package" true "$(judge schedule "$base" "$base" | sed -n 1p)"
check "a workflow dispatch builds the package" true "$(judge workflow_dispatch "$base" "$base" | sed -n 1p)"
check "an empty range builds the package" true "$(judge pull_request "$base" "$base" | sed -n 1p)"

# Take the rule away and the answer has to change. A rule whose removal
# changes nothing was not what decided the verdict.
head="$(change tests/scripts/shell/workflow_behaviour/debian-ci-inputs.test.sh)"
mutant="$work/judge-without-the-script-rule.sh"
sed '/tests\/scripts.*) ;;/d' "$script" >"$mutant"
out="$(judge pull_request "$base" "$head" "$mutant")"
check "removing the script-only rule builds the package" true "$(printf '%s\n' "$out" | sed -n 1p)"

head="$(change internal/lib.rs)"
mutant="$work/judge-without-the-crate-rule.sh"
sed '/internal\/\*) answer true/d' "$script" >"$mutant"
out="$(judge pull_request "$base" "$head" "$mutant")"
check "removing the crate rule changes the reason" 0 \
	"$(printf '%s\n' "$out" | sed -n 2p | grep -cF 'changes the crate')"

# The workflow reads the judge from the reviewed base, never from the pull
# request's own tree, and both jobs ask it.
# shellcheck disable=SC2016
check "two jobs read the judge from the base" 2 \
	"$(grep -c 'git show "\$BASE_SHA:.github/scripts/checks/debian-ci-inputs.sh"' "$workflow")"
check "the judge is never read from the working tree" 0 \
	"$(grep -c 'bash .github/scripts/checks/debian-ci-inputs.sh' "$workflow")"
# shellcheck disable=SC2016
check "the range comes from the event" 1 \
	"$(grep -c '^  BASE_SHA: \${{ github.event.pull_request.base.sha' "$workflow")"
check "every heavy step is behind the guard" 10 \
	"$(grep -c "steps.decide.outputs.run != 'false'" "$workflow")"
check "the package job keeps its name" 1 \
	"$(grep -c '^    name: dpkg-buildpackage$' "$workflow")"
check "the vendored job keeps its name" 1 \
	"$(grep -c '^    name: build (vendored / offline)$' "$workflow")"
check "the caller keeps its job id" 1 "$(grep -c '^  debian:$' "$caller")"

# The caller watches the classifier, or the rule that names it is dead.
check "the caller watches the classifier" 2 \
	"$(grep -c '"\.github/scripts/checks/debian-ci-inputs\.sh"' "$caller")"
# The lane still watches the whole test tree: the classifier decides whether
# to build, it never decides whether to run. #1295.
check "the caller still watches the test tree" 2 "$(grep -c '"tests/\*\*"' "$caller")"

echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
