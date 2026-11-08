#!/usr/bin/env bash
# Exercise the judge that lets the Rust lanes skip when nothing they read changed.
#
# The judge is `.github/scripts/checks/rust-ci-inputs.sh`, read by the
# workflow from the reviewed base commit. It answers one question: does this
# range touch anything the Rust lint, test, coverage and extra-platform lanes
# read? Exit 0 means the lanes may be skipped, exit 1 means they must run.
#
# Every rule in the judge has a case below, and every case names the rule it
# expects, not only the verdict. Removing a path from the judge's list of
# paths that force the lanes to run changes the reason the judge gives, and
# this file fails: a rule nobody exercises is a rule that can be deleted
# without anybody noticing, which is how a lane stops running without a
# decision.
set -uo pipefail
export LC_ALL=C
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
script="$HERE/.github/scripts/checks/rust-ci-inputs.sh"
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

# judge <event> <base> <head>: the verdict, the reason and the exit code,
# one per line.
judge() { # <event> <base> <head>
	local out="$work/output" reason rc=0
	: >"$out"
	reason="$(
		cd "$work" || exit 1
		GITHUB_OUTPUT="$out" EVENT="$1" bash "$script" "$2" "$3"
	)" || rc=$?
	printf '%s\n%s\n%s\n' "$(sed -n 's/^run=//p' "$out")" "$reason" "$rc"
}

# must_run <reason> <path>...: the lanes run, and this is why.
must_run() { # <reason> <path>...
	local reason="$1" path head out
	shift
	for path in "$@"; do
		head="$(change "$path")"
		out="$(judge pull_request "$base" "$head")"
		check "classify $path runs the lanes" true "$(printf '%s\n' "$out" | sed -n 1p)"
		check "classify $path exits 1" 1 "$(printf '%s\n' "$out" | sed -n 3p)"
		check "classify $path names its rule" 1 \
			"$(printf '%s\n' "$out" | sed -n 2p | grep -cF "$reason")"
	done
}

# may_skip <path>...: the lanes may be skipped, and the judge says so.
may_skip() { # <path>...
	local path head out
	for path in "$@"; do
		head="$(change "$path")"
		out="$(judge pull_request "$base" "$head")"
		check "classify $path skips the lanes" false "$(printf '%s\n' "$out" | sed -n 1p)"
		check "classify $path exits 0" 0 "$(printf '%s\n' "$out" | sed -n 3p)"
		check "classify $path says nothing reaches them" 1 \
			"$(printf '%s\n' "$out" | sed -n 2p | grep -cF 'nothing in this range can reach the Rust lanes')"
	done
}

# mixed <reason> <path>...: several paths changed at once, one of which may
# be skipped and one of which may not.
mixed() { # <reason> <path>...
	local reason="$1" head out
	shift
	head="$(change "$@")"
	out="$(judge pull_request "$base" "$head")"
	check "classify $* runs the lanes" true "$(printf '%s\n' "$out" | sed -n 1p)"
	check "classify $* exits 1" 1 "$(printf '%s\n' "$out" | sed -n 3p)"
	check "classify $* names its rule" 1 \
		"$(printf '%s\n' "$out" | sed -n 2p | grep -cF "$reason")"
}

# Every path the Rust lanes read, and every path whose effect on them
# nobody measured.
must_run "changes the classifier" .github/scripts/checks/rust-ci-inputs.sh
must_run "changes the dependency graph" Cargo.toml Cargo.lock fuzz/Cargo.toml bench/timeit/Cargo.lock
must_run "changes the crate" internal/lib.rs internal/backends/engine/observe/stats/stats.rs
must_run "changes the build script" build.rs
must_run "changes what fmt and clippy do" rustfmt.toml .cargo/config.toml
must_run "changes the test suite" \
	tests/hostile/input/hostile_compose.rs tests/fixtures/interpolation/ordinary-config.yml \
	tests/scripts/stray.rs tests/scripts/shell/notes.md
must_run "changes the packaging" debian/rules debian/control
must_run "changes the CI" .github/workflows/ci.yml .github/workflows/reusable-rust-ci.yml
must_run "changes the checks CI runs" .github/scripts/checks/check-planning-words.sh
must_run "is read by the repository tests" \
	README.md CONTRIBUTING.md docs/commands.md docs/commands-reference.md \
	docs/self-update.md fuzz/seeds/parse_compose/multi-document.yml \
	bench/scenarios/build/Dockerfile
must_run "is read by the release-key test" install.sh install.ps1
must_run "is read by the supply-chain test" deny.toml .github/dependabot.yml
must_run "is not a known input of the Rust lanes" unknown.txt

# One path that may be skipped does not excuse one that may not.
mixed "changes the crate" tests/scripts/shell/workflow_behaviour/rust-ci-inputs.test.sh internal/lib.rs
mixed "changes the CI" docs/assets/demo.sh .github/workflows/ci.yml

# Skip: what no Rust lane and no Rust test reads.
may_skip LICENSE .editorconfig about.hbs about.toml \
	.github/CODEOWNERS .github/PULL_REQUEST_TEMPLATE.md \
	.github/ISSUE_TEMPLATE/bug_report.md \
	docs/assets/demo.sh docs/assets/bench.svg \
	tests/scripts/shell/workflow_behaviour/rust-ci-inputs.test.sh
change LICENSE .editorconfig about.hbs
out="$(judge pull_request "$base" "$(git -C "$work" rev-parse HEAD)")"
check "several skippable paths skip the lanes" false "$(printf '%s\n' "$out" | sed -n 1p)"

# Fail closed: no range, no history, an event that cannot skip, an empty
# range. Every one of them runs the lanes.
check "no range runs the lanes" true "$(judge pull_request '' '' | sed -n 1p)"
check "an unknown base runs the lanes" true "$(judge pull_request deadbeef "$base" | sed -n 1p)"
check "an unknown head runs the lanes" true "$(judge pull_request "$base" deadbeef | sed -n 1p)"
check "a schedule runs the lanes" true "$(judge schedule "$base" "$base" | sed -n 1p)"
check "a workflow dispatch runs the lanes" true "$(judge workflow_dispatch "$base" "$base" | sed -n 1p)"
check "an empty range runs the lanes" true "$(judge pull_request "$base" "$base" | sed -n 1p)"

# The workflow reads the judge from the reviewed base, never from the pull
# request's own tree. Two jobs ask it: `classify`, whose answer the jobs
# skipped whole read, and `lint`, which skips only its heavy steps.
# shellcheck disable=SC2016
check "two jobs read the judge from the base" 2 \
	"$(grep -c 'git show "\$BASE_SHA:.github/scripts/checks/rust-ci-inputs.sh"' "$workflow")"
check "the judge is never read from the working tree" 0 \
	"$(grep -c 'bash .github/scripts/checks/rust-ci-inputs.sh' "$workflow")"
# shellcheck disable=SC2016
check "the range comes from the event" 1 \
	"$(grep -c '^  BASE_SHA: \${{ github.event.pull_request.base.sha' "$workflow")"
check "a missing step decision runs the lanes" 4 \
	"$(grep -c "steps.decide.outputs.run != 'false'" "$workflow")"
check "a missing job decision runs the lanes" 3 \
	"$(grep -c "needs.classify.outputs.run != 'false'" "$workflow")"
check "a classifier that failed runs the lanes" 3 \
	"$(grep -c "needs.classify.result != 'success'" "$workflow")"

echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
