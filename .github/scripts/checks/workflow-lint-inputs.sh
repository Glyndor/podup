#!/usr/bin/env bash
#
# Usage: workflow-lint-inputs.sh BASE_SHA HEAD_SHA
#
# Decide whether the workflow lint has to run for a pull request's range.
#
# The lint installs Go, installs actionlint, downloads a pinned shellcheck
# and then reads every workflow in the tree. That is a minute of runner
# time and a download of a third-party binary on every pull request, and a
# pull request that touches no workflow pays all of it for nothing.
#
# Reads $EVENT (the GitHub event name) and writes `run=true` or `run=false`
# to $GITHUB_OUTPUT. Exit 0 means the lint may be skipped, exit 1 means it
# must run. The workflow reads the OUTPUT rather than the exit code, because
# a missing output has to mean "run" and never "skip": the classifier is the
# only thing standing between a broken checkout and a lint nobody ran.
#
# The workflow runs this script from the reviewed base commit, never from the
# pull request's own tree, so a pull request cannot rewrite the judge of its
# own diff. Every path below is therefore reviewed policy.
#
# What forces the lint to run:
#
#   .github/workflows/              the files actionlint and every assertion read
#   .github/scripts/                the assertions the lint runs
#   .github/actionlint.yaml, .yml   actionlint's own configuration
#   tests/repository/workflows/     the tests that read those workflows
#   tests/scripts/shell/workflow_*  the shell tests that drive those steps
#   Cargo.toml                      the manifest that declares those tests
#
# Only a closed list of paths may skip the lint, and every other path runs it:
#
#   internal/            the crate; actionlint never opens a Rust file
#   docs/, README.md, CONTRIBUTING.md, LICENSE   prose and licence
#   tests/hostile/, tests/cli/, tests/engine/    Rust integration tests that
#                        the lint neither reads nor drives
#   .editorconfig
#
# The default is "run". An earlier version skipped every path it did not
# know, and an independent review showed the cost: a path git quotes (unicode,
# a tab, a newline) matched nothing and fell to the skip, so a workflow named
# with an accent skipped the lint while actionlint still read it. Paths are
# now read NUL-separated, so nothing is quoted, and a path nobody listed
# runs the lint, the way the Rust lanes treat theirs.
set -uo pipefail
export LC_ALL=C

base="${1:-}"
head="${2:-}"
output="${GITHUB_OUTPUT:-}"

answer() { # <true|false> <reason>
	if [ -n "$output" ]; then
		echo "run=$1" >>"$output"
	fi
	echo "$2"
	if [ "$1" = false ]; then
		exit 0
	fi
	exit 1
}

if [ -z "$base" ] || [ -z "$head" ]; then
	answer true "no range to classify; the workflow lint runs"
fi
case "${EVENT:-}" in
	pull_request | push | merge_group) ;;
	*) answer true "event ${EVENT:-unknown} cannot skip; the workflow lint runs" ;;
esac
# NUL-separated, so a path with a quote, a tab or a newline arrives as it is.
changed="$(git diff -z --name-only --no-renames "$base" "$head" 2>/dev/null | tr '\0' '\001')" ||
	answer true "cannot read $base..$head; the workflow lint runs"
[ -n "$changed" ] || answer true "the range is empty; the workflow lint runs"

while IFS= read -r -d $'\001' path; do
	case "$path" in
		# The judge itself: a pull request that edits this script must not
		# be judged by its own edit.
		.github/scripts/checks/workflow-lint-inputs.sh) answer true "$path changes the classifier" ;;
		# The workflows the lint reads.
		.github/workflows/*) answer true "$path changes a workflow the lint reads" ;;
		# The checks the lint runs over them.
		.github/scripts/*) answer true "$path changes a check the lint runs" ;;
		# actionlint's own configuration.
		.github/actionlint.yaml | .github/actionlint.yml) answer true "$path changes the actionlint configuration" ;;
		# The tests that read the workflows, and the manifest that declares
		# them.
		tests/repository/workflows/*) answer true "$path changes a test that reads the workflows" ;;
		tests/scripts/shell/workflow_*) answer true "$path changes a test that reads the workflows" ;;
		Cargo.toml) answer true "$path changes the manifest that declares those tests" ;;
		# What the lint never reads: the only paths that may skip it.
		internal/* | docs/* | README.md | CONTRIBUTING.md | LICENSE | .editorconfig) ;;
		tests/hostile/* | tests/cli/* | tests/engine/*) ;;
		# A path nobody listed, or one the shell cannot match, runs the lint.
		*) answer true "$path is not a path that may skip the workflow lint" ;;
	esac
done <<<"$changed"

answer false "nothing in this range can reach the workflow lint"
