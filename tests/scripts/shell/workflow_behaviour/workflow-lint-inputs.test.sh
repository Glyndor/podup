#!/usr/bin/env bash
# Exercise the judge that lets the workflow lint skip when nothing it reads changed.
#
# The judge is `.github/scripts/checks/workflow-lint-inputs.sh`, read by the
# workflow from the reviewed base commit. It answers one question: does this
# range touch anything the lint reads? Exit 0 means the lint may be skipped,
# exit 1 means it must run.
#
# Every rule in the judge has a case below, and every case names the rule it
# expects, not only the verdict. Removing a path from the judge's list of
# paths that force the lint to run changes the reason the judge gives, and
# this file fails: a rule nobody exercises is a rule that can be deleted
# without anybody noticing, which is how a lint stops running without a
# decision.
#
# The last part of the file is the mutation check: each rule is removed from
# a copy of the judge, and the copy has to answer differently for the paths
# that rule covers. A rule whose removal changes nothing is not a rule; it is
# a comment.
set -uo pipefail
export LC_ALL=C
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
script="$HERE/.github/scripts/checks/workflow-lint-inputs.sh"
workflow="$HERE/.github/workflows/reusable-workflow-lint.yml"
caller="$HERE/.github/workflows/workflow-lint.yml"
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
# code, one per line. The fourth argument is the judge to run, so a copy of
# it can be judged the same way.
judge() { # <event> <base> <head> [judge]
	local event="$1" from="$2" to="$3" which="${4:-$script}"
	local out="$work/output" reason rc=0
	: >"$out"
	reason="$(
		cd "$work" || exit 1
		GITHUB_OUTPUT="$out" EVENT="$event" bash "$which" "$from" "$to"
	)" || rc=$?
	printf '%s\n%s\n%s\n' "$(sed -n 's/^run=//p' "$out")" "$reason" "$rc"
}

# must_run <reason> <path>...: the lint runs, and this is why.
must_run() { # <reason> <path>...
	local reason="$1" path head out
	shift
	for path in "$@"; do
		head="$(change "$path")"
		out="$(judge pull_request "$base" "$head")"
		check "classify $path runs the lint" true "$(printf '%s\n' "$out" | sed -n 1p)"
		check "classify $path exits 1" 1 "$(printf '%s\n' "$out" | sed -n 3p)"
		check "classify $path names its rule" 1 \
			"$(printf '%s\n' "$out" | sed -n 2p | grep -cF "$reason")"
	done
}

# may_skip <path>...: the lint may be skipped, and the judge says so.
may_skip() { # <path>...
	local path head out
	for path in "$@"; do
		head="$(change "$path")"
		out="$(judge pull_request "$base" "$head")"
		check "classify $path skips the lint" false "$(printf '%s\n' "$out" | sed -n 1p)"
		check "classify $path exits 0" 0 "$(printf '%s\n' "$out" | sed -n 3p)"
		check "classify $path says nothing reaches it" 1 \
			"$(printf '%s\n' "$out" | sed -n 2p | grep -cF 'nothing in this range can reach the workflow lint')"
	done
}

# mutate <fragment> <path>: the copy of the judge without the rule that
# names <fragment> has to answer differently for <path>. The copy still has
# to be a script, so a removal that breaks the judge is a removal nobody can
# make either.
mutate() { # <fragment> <path>
	local fragment="$1" path="$2" head before after
	head="$(change "$path")"
	before="$(judge pull_request "$base" "$head")"
	grep -v -F "$fragment" "$script" >"$work/mutant.sh"
	after="$(judge pull_request "$base" "$head" "$work/mutant.sh")"
	check "the judge without the rule for $path still parses" 0 \
		"$(bash -n "$work/mutant.sh" 2>/dev/null; echo $?)"
	check "the rule for $path is load-bearing" 1 \
		"$([ "$before" != "$after" ] && echo 1 || echo 0)"
}

# Every path the lint reads, and every path whose effect on it nobody
# measured.
must_run "changes the classifier" .github/scripts/checks/workflow-lint-inputs.sh
must_run "changes a workflow the lint reads" \
	.github/workflows/ci.yml .github/workflows/reusable-workflow-lint.yml
must_run "changes a check the lint runs" \
	.github/scripts/checks/check-caller-with-defaults.py \
	.github/scripts/checks/check-merge-group-triggers.py \
	.github/scripts/checks/check-local-reusables.py
must_run "changes the actionlint configuration" \
	.github/actionlint.yaml .github/actionlint.yml
must_run "changes a test that reads the workflows" \
	tests/repository/workflows/workflow_rust_ci_classifier.rs \
	tests/scripts/shell/workflow_lint/reusable-workflow-lint-caller-if.test.sh
must_run "changes the manifest that declares those tests" Cargo.toml

# Skip: what the lint never reads, and only a closed list of it.
may_skip internal/lib.rs internal/backends/engine/lib.rs \
	README.md CONTRIBUTING.md docs/commands.md docs/assets/demo.svg LICENSE .editorconfig \
	tests/hostile/parsing/hostile_compose.rs tests/cli/output/build_contract.rs \
	tests/engine/engine_integration.rs

# The default is "run": a path nobody listed runs the lint, and so does a path
# git would quote. An earlier version skipped both, and a workflow with an
# accent in its name skipped the lint that still reads it.
must_run "is not a path that may skip the workflow lint" \
	unknown.txt Cargo.lock build.rs install.sh debian/rules deny.toml \
	.github/dependabot.yml .github/CODEOWNERS about.hbs
must_run "changes a workflow the lint reads" \
	".github/workflows/é.yml" ".github/workflows/with space.yml" \
	".github/workflows/tab"$'\t'"here.yml"
# A newline in the name splits the reason over two lines, which the helper
# above cannot read, so this one asserts the verdict only.
head="$(change ".github/workflows/a"$'\n'"b.yml")"
check "a workflow with a newline in its name runs the lint" true \
	"$(judge pull_request "$base" "$head" | sed -n 1p)"
must_run "changes a check the lint runs" \
	".github/scripts/é.py" ".github/scripts/checks/tab"$'\t'"here.py"

# Each rule carries the verdict it is there for. Removing it has to change
# the answer for the paths it covers.
mutate ".github/scripts/checks/workflow-lint-inputs.sh) answer true" \
	.github/scripts/checks/workflow-lint-inputs.sh
mutate ".github/workflows/*) answer true" .github/workflows/ci.yml
mutate ".github/scripts/*) answer true" \
	.github/scripts/checks/check-caller-with-defaults.py
mutate ".github/actionlint.yaml | .github/actionlint.yml) answer true" \
	.github/actionlint.yaml
mutate "tests/repository/workflows/*) answer true" \
	tests/repository/workflows/workflow_rust_ci_classifier.rs
mutate "tests/scripts/shell/workflow_*) answer true" \
	tests/scripts/shell/workflow_lint/reusable-workflow-lint-caller-if.test.sh
mutate "Cargo.toml) answer true" Cargo.toml
# shellcheck disable=SC2016 # the fragment is matched literally, $path included
mutate '*) answer true "$path is not a path that may skip the workflow lint" ;;' unknown.txt

# Fail closed: no range, no history, an event that cannot skip, an empty
# range. Every one of them runs the lint.
check "no range runs the lint" true "$(judge pull_request '' '' | sed -n 1p)"
check "an unknown base runs the lint" true "$(judge pull_request deadbeef "$base" | sed -n 1p)"
check "an unknown head runs the lint" true "$(judge pull_request "$base" deadbeef | sed -n 1p)"
check "a schedule runs the lint" true "$(judge schedule "$base" "$base" | sed -n 1p)"
check "a workflow dispatch runs the lint" true "$(judge workflow_dispatch "$base" "$base" | sed -n 1p)"
check "an empty range runs the lint" true "$(judge pull_request "$base" "$base" | sed -n 1p)"

# The workflow reads the judge from the reviewed base, never from the pull
# request's own tree, and every step that installs a tool or reads a
# workflow is behind the guard.
# shellcheck disable=SC2016
check "the judge is read from the base" 1 \
	"$(grep -c 'git show "\$BASE_SHA:.github/scripts/checks/workflow-lint-inputs.sh"' "$workflow")"
check "the judge is never read from the working tree" 0 \
	"$(grep -c 'bash .github/scripts/checks/workflow-lint-inputs.sh' "$workflow")"
check "every step behind the guard carries it" 11 \
	"$(grep -c "steps.decide.outputs.run != 'false'" "$workflow")"
check "the checkout fetches the history the judge reads" 1 \
	"$(grep -c 'fetch-depth: 0' "$workflow")"

# The check name the ruleset requires does not move: the caller names the
# job `workflow-lint` and the reusable's job is named `workflow-lint`.
check "the caller's job id is still workflow-lint" 1 \
	"$(grep -c '^  workflow-lint:$' "$caller")"
check "the reusable's job is still named workflow-lint" 1 \
	"$(grep -c '^    name: workflow-lint$' "$workflow")"

echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
