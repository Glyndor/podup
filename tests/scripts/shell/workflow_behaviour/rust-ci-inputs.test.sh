#!/usr/bin/env bash
# Exercise the judge that lets the Rust lanes skip when nothing they read changed.
#
# The judge is `.github/scripts/checks/rust-ci-inputs.sh`, read by the
# workflow from the reviewed base commit. It answers with two outputs: `run`
# says whether the Rust lint, test, coverage and extra-platform lanes have
# anything to do, and `docs_only` says whether the range is documentation
# alone, in which case only the tests that read a document run.
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

# judge <event> <base> <head>: the two answers, the reason and the exit code,
# one per line.
judge() { # <event> <base> <head>
	local out="$work/output" reason rc=0
	: >"$out"
	reason="$(
		cd "$work" || exit 1
		GITHUB_OUTPUT="$out" EVENT="$1" bash "$script" "$2" "$3"
	)" || rc=$?
	printf '%s\n%s\n%s\n%s\n%s\n' \
		"$(sed -n 's/^run=//p' "$out")" \
		"$(sed -n 's/^docs_only=//p' "$out")" \
		"$reason" "$rc" \
		"$(sed -n 's/^ci_only=//p' "$out")"
}

# ci_only <path>...: the range is the CI alone (or the CI and documents), so
# the Linux suite runs and coverage, the other platforms and the lint do not.
ci_only() { # <path>...
	local head out
	head="$(change "$@")"
	out="$(judge pull_request "$base" "$head")"
	check "classify $* runs the lanes" true "$(printf '%s\n' "$out" | sed -n 1p)"
	check "classify $* is not documentation only" false "$(printf '%s\n' "$out" | sed -n 2p)"
	check "classify $* exits 3" 3 "$(printf '%s\n' "$out" | sed -n 4p)"
	check "classify $* is the CI-only answer" true "$(printf '%s\n' "$out" | sed -n 5p)"
}

# must_run <reason> <path>...: the lanes run, and this is why.
must_run() { # <reason> <path>...
	local reason="$1" path head out
	shift
	for path in "$@"; do
		head="$(change "$path")"
		out="$(judge pull_request "$base" "$head")"
		check "classify $path runs the lanes" true "$(printf '%s\n' "$out" | sed -n 1p)"
		check "classify $path is not documentation only" false "$(printf '%s\n' "$out" | sed -n 2p)"
		check "classify $path exits 1" 1 "$(printf '%s\n' "$out" | sed -n 4p)"
		check "classify $path is not the CI-only answer" false "$(printf '%s\n' "$out" | sed -n 5p)"
		check "classify $path names its rule" 1 \
			"$(printf '%s\n' "$out" | sed -n 3p | grep -cF "$reason")"
	done
}

# may_skip <path>...: the lanes may be skipped, and the judge says so.
may_skip() { # <path>...
	local path head out
	for path in "$@"; do
		head="$(change "$path")"
		out="$(judge pull_request "$base" "$head")"
		check "classify $path skips the lanes" false "$(printf '%s\n' "$out" | sed -n 1p)"
		check "classify $path is not documentation only" false "$(printf '%s\n' "$out" | sed -n 2p)"
		check "classify $path exits 0" 0 "$(printf '%s\n' "$out" | sed -n 4p)"
		check "classify $path says nothing reaches them" 1 \
			"$(printf '%s\n' "$out" | sed -n 3p | grep -cF 'nothing in this range can reach the Rust lanes')"
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
	check "classify $* is not documentation only" false "$(printf '%s\n' "$out" | sed -n 2p)"
	check "classify $* exits 1" 1 "$(printf '%s\n' "$out" | sed -n 4p)"
	check "classify $* names its rule" 1 \
		"$(printf '%s\n' "$out" | sed -n 3p | grep -cF "$reason")"
}

# docs_only <path>...: the range is documentation alone, so the focused lane
# runs and nothing else does.
docs_only() { # <path>...
	local path head out
	for path in "$@"; do
		head="$(change "$path")"
		out="$(judge pull_request "$base" "$head")"
		check "classify $path runs the lanes" true "$(printf '%s\n' "$out" | sed -n 1p)"
		check "classify $path is documentation only" true "$(printf '%s\n' "$out" | sed -n 2p)"
		check "classify $path exits 2" 2 "$(printf '%s\n' "$out" | sed -n 4p)"
		check "classify $path names the focused lane" 1 \
			"$(printf '%s\n' "$out" | sed -n 3p | grep -cF 'focused documentation lane')"
	done
}

# docs_and_more <reason> <path>...: a document next to anything else runs
# every lane. The reason is the rule of the path that forces it: a path that
# forces the lanes on its own says so, and a path that only may be skipped
# makes the range a mixed one.
docs_and_more() { # <reason> <path>...
	local reason="$1" head out
	shift
	head="$(change "$@")"
	out="$(judge pull_request "$base" "$head")"
	check "classify $* runs every lane" true "$(printf '%s\n' "$out" | sed -n 1p)"
	check "classify $* is not documentation only" false "$(printf '%s\n' "$out" | sed -n 2p)"
	check "classify $* exits 1" 1 "$(printf '%s\n' "$out" | sed -n 4p)"
	check "classify $* names its rule" 1 \
		"$(printf '%s\n' "$out" | sed -n 3p | grep -cF "$reason")"
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
must_run "changes the pipeline of the Rust lanes" \
	.github/workflows/ci.yml .github/workflows/reusable-rust-ci.yml .github/scripts/checks/rust-build-inputs.sh
must_run "is read by the repository tests" \
	fuzz/seeds/parse_compose/multi-document.yml bench/scenarios/build/Dockerfile
must_run "is read by the release-key test" install.sh install.ps1
must_run "is read by the supply-chain test" deny.toml
must_run "is not a known input of the Rust lanes" unknown.txt

# One path that may be skipped does not excuse one that may not.
mixed "changes the crate" tests/scripts/shell/workflow_behaviour/rust-ci-inputs.test.sh internal/lib.rs
mixed "changes the pipeline of the Rust lanes" docs/assets/demo.sh .github/workflows/ci.yml
mixed "changes the crate" .github/workflows/lint-shell.yml internal/lib.rs
mixed "changes the test suite" .github/scripts/checks/folder-limit.sh tests/hostile/input/hostile_compose.rs

# The CI-only answer: the files of the CI and nothing the crate reads.
ci_only .github/workflows/lint-shell.yml
ci_only .github/workflows/podman-lane.yml .github/scripts/checks/folder-limit.sh
ci_only .github/scripts/release/sign.py .github/scripts/hygiene/check-secrets.sh
ci_only .github/dependabot.yml
ci_only .github/workflows/dco.yml docs/commands.md
ci_only .github/workflows/audit.yml tests/scripts/shell/workflow_behaviour/rust-ci-inputs.test.sh LICENSE

# The third answer: a range of documents alone. The focused lane runs, and
# only the tests that read a document run in it.
docs_only README.md CONTRIBUTING.md docs/commands.md docs/commands-reference.md \
	docs/self-update.md docs/debian-packaging.md

# A name Windows cannot check out must reach the Windows leg, in any directory,
# with any extension and in any case; a name that only starts like one stays.
for reserved in docs/CON.md docs/aux.md docs/Nul.md docs/COM1.md docs/lpt9.md docs/PRN.x.md docs/sub/AUX.md docs/CON/page.md; do
	mkdir -p "$work/$(dirname "$reserved")"
	must_run "name Windows cannot check out" "$reserved"
done
docs_only docs/CONSOLE.md docs/COM10.md docs/auxiliary.md

# A document next to anything else runs every lane: the two answers differ
# in what they skip, and the safe reading of a mixed range is the whole thing.
docs_and_more "changes the crate" docs/commands.md internal/lib.rs
docs_and_more "changes the dependency graph" README.md Cargo.toml
docs_and_more "documentation and something else" docs/commands.md LICENSE
docs_and_more "documentation and something else" \
	docs/commands.md tests/scripts/shell/workflow_behaviour/rust-ci-inputs.test.sh

# The images under docs/assets are skipped as they always were, and a document
# next to one of them still decides the lane.
head="$(change docs/assets/bench.svg docs/commands.md)"
out="$(judge pull_request "$base" "$head")"
check "a document next to an image is documentation only" true "$(printf '%s\n' "$out" | sed -n 2p)"
check "a document next to an image exits 2" 2 "$(printf '%s\n' "$out" | sed -n 4p)"

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
check "an empty range is not documentation only" false \
	"$(judge pull_request "$base" "$base" | sed -n 2p)"
check "an empty range exits 1" 1 "$(judge pull_request "$base" "$base" | sed -n 4p)"

# The workflow reads the judge from the reviewed base, never from the pull
# request's own tree. One job asks it, `classify`, and every other job reads
# its answer: the jobs skipped whole and the heavy steps of `lint`.
# shellcheck disable=SC2016
check "one job reads the judge from the base" 1 \
	"$(grep -c 'git show "\$BASE_SHA:.github/scripts/checks/rust-ci-inputs.sh"' "$workflow")"
check "the judge is never read from the working tree" 0 \
	"$(grep -c 'bash .github/scripts/checks/rust-ci-inputs.sh' "$workflow")"
# shellcheck disable=SC2016
check "the range comes from the event" 1 \
	"$(grep -c '^  BASE_SHA: \${{ github.event.pull_request.base.sha' "$workflow")"
check "no step decides for itself" 0 \
	"$(grep -c "steps.decide.outputs.run != 'false'" "$workflow")"
check "a missing decision runs the lanes (three jobs and four steps)" 7 \
	"$(grep -c "needs.classify.outputs.run != 'false'" "$workflow")"
check "a classifier that failed runs the lanes" 5 \
	"$(grep -c "needs.classify.result != 'success'" "$workflow")"

# The third answer has to reach the jobs that read it.
# shellcheck disable=SC2016
check "the classifier exposes the documentation answer" 1 \
	"$(grep -c 'docs_only: ${{ steps.decide.outputs.docs_only }}' "$workflow")"
check "the lint job's heavy steps skip a documentation-only or CI-only range" 4 \
	"$(grep -c "needs.classify.outputs.run != 'false' && !(needs.classify.outputs.run == 'true' && (needs.classify.outputs.docs_only == 'true' || needs.classify.outputs.ci_only == 'true'))" "$workflow")"
check "the matrix and the coverage skip a documentation-only or CI-only range" 2 \
	"$(grep -c "needs.classify.outputs.run != 'false') && !(needs.classify.result == 'success' && needs.classify.outputs.run == 'true' && (needs.classify.outputs.docs_only == 'true' || needs.classify.outputs.ci_only == 'true')) }}" "$workflow")"
# shellcheck disable=SC2016
check "the classifier exposes the CI-only answer" 1 \
	"$(grep -c 'ci_only: ${{ steps.decide.outputs.ci_only }}' "$workflow")"
check "the full suite runs unless a healthy classifier said documentation only" 1 \
	"$(grep -c "if: \${{ !(needs.classify.result == 'success' && needs.classify.outputs.run == 'true' && needs.classify.outputs.docs_only == 'true') }}" "$workflow")"
check "the focused lane runs only for a healthy documentation-only answer" 1 \
	"$(grep -c "if: needs.classify.result == 'success' && needs.classify.outputs.run == 'true' && needs.classify.outputs.docs_only == 'true'" "$workflow")"

# The focused lane has to be every test that reads a document, not the two
# the classifier's comment names first. A document that breaks a test outside
# that list is a document that merges broken.
focused="$(grep 'cargo test --locked --all-features --lib' "$workflow")"
check "the focused lane runs the unit tests that read a document" 1 \
	"$(printf '%s' "$focused" | grep -c -- '--all-features --lib')"
while IFS='|' read -r file target; do
	[ -n "$file" ] || continue
	check "$file still reads a document" yes \
		"$(grep -qE 'docs/|README\.md|CONTRIBUTING\.md' "$HERE/$file" && echo yes || echo no)"
	check "the focused lane runs $target" 1 \
		"$(printf '%s' "$focused" | grep -c -- "--test $target")"
done <<EOF
tests/repository/docs/docs_links.rs|docs_links
tests/repository/docs/docs_flags.rs|docs_flags
tests/repository/packaging/podup_is_not_published.rs|podup_is_not_published
tests/repository/packaging/no_system_wide_apt_policy.rs|no_system_wide_apt_policy
tests/repository/packaging/keyring_bootstrap_order.rs|keyring_bootstrap_order
EOF

# This one reads every tracked file, so it names no document and the loop above
# cannot find it; the shell scan that replaces it ignores binary-looking files,
# and a README with a NUL byte would pass that scan and fail this test.
check "the focused lane runs no_private_document_references" 1 \
	"$(printf '%s' "$focused" | grep -c -- '--test no_private_document_references')"

# Files that may skip the lanes are still read by the Rust test that scans
# every tracked file for a private document tree, so `lint` runs the same scan
# with no classifier in front of it.
check "lint runs the private-document scan once" 1 \
	"$(grep -c 'bash .github/scripts/checks/check-private-document-references.sh' "$workflow")"
check "the private-document scan carries no condition" 0 \
	"$(grep -B3 'check-private-document-references.sh' "$workflow" | grep -c ' if:')"

echo "$pass passed, $fail failed"
printf 'DONE %s %s %s\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
